//! `stage commit`: a commit on the run's branch, never on the main branch,
//! of a stage card in review, with `commit=<sha>` logged on the card.

use std::path::Path;

use crate::card::board::{self, CardId, State};
use crate::card::typed::Name;
use crate::git;
use crate::hook::attribution;
use crate::refusal::Refusal;
use crate::run::gauntlet;
use crate::store::Board;

/// Why nothing was committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitRefused {
    /// HEAD is on the main branch, named.
    Main(String),
    /// No board, or no such card.
    Card,
    /// The card is not in review: its state.
    State(State),
    /// HEAD is not the run's branch, named.
    Branch(String),
    /// A rebase is in progress in the worktree.
    Rebasing,
    /// `git commit` failed.
    Git,
    /// The index equals HEAD: HEAD's short sha, and whether the worktree holds changes.
    NothingStaged { head: String, dirty: bool },
    /// `--adopt`: the worktree holds changes.
    Dirty,
    /// `--adopt`: the sha is not HEAD; HEAD's short sha.
    NotHead { head: String },
    /// `--adopt`: HEAD has this many parents, not one.
    Merge { parents: usize },
    /// `--adopt`: HEAD is already logged as `commit=` on this card of the run.
    Adopted { card: String },
    /// `--adopt`: this commit between the branch point and HEAD^ is logged on no card (short sha).
    Unlogged { sha: String },
    /// `--adopt`: the branch point of HEAD with the base branch cannot be found.
    NoMergeBase,
    /// `--adopt`: the commit touches this path outside the card's `files` list.
    Files { path: String },
    /// `--adopt`: the message carries attribution trailers.
    Trailers,
}

impl CommitRefused {
    /// The `commit=refused` line; `card` is the stage card asked for.
    #[must_use]
    pub fn refusal(&self, card: Option<&str>) -> Refusal {
        match self {
            CommitRefused::Main(main) => Refusal::new("commit", "main")
                .field("branch", main)
                .hint(format!(
                    "commit on the run's branch, not {main}: `ferrule gate approve <slug> gate2` creates it and its worktree; with --no-run, check out another branch first"
                )),
            CommitRefused::Card => Refusal::card("commit"),
            CommitRefused::State(st) => Refusal::new("commit", "state")
                .field("state", st)
                .hint(format!(
                    "move the card to inreview first: `ferrule board move {} inreview`",
                    card.unwrap_or("<card>")
                )),
            CommitRefused::Branch(want) => Refusal::new("commit", "branch")
                .field("want", want)
                .hint(format!(
                    "check out {want}, the run's branch, and commit there"
                )),
            CommitRefused::Rebasing => Refusal::new("commit", "rebasing").hint(
                "a rebase is in progress in the worktree: resolve and GIT_EDITOR=true git rebase --continue, then ferrule run rebase <slug>; or git rebase --abort",
            ),
            CommitRefused::Git => Refusal::new("commit", "git")
                .hint("git commit failed; read the message git prints on stderr"),
            CommitRefused::NothingStaged { head, dirty } => {
                Refusal::new("commit", "nothing-staged")
                    .field("head", head)
                    .hint(if *dirty {
                        "the worktree holds changes that are not staged: stage them with `git add`, then commit again".to_owned()
                    } else {
                        format!(
                            "the worktree is clean: the changes may already be committed, check `git log` and report head={head} if they are"
                        )
                    })
            }
            CommitRefused::Dirty => Refusal::new("commit", "dirty").hint(format!(
                "the worktree holds changes: commit them with `ferrule stage commit {} -m <msg>` or discard them",
                card.unwrap_or("<card>")
            )),
            CommitRefused::NotHead { head } => Refusal::new("commit", "not-head")
                .field("head", head)
                .hint(format!(
                    "adopt the branch's HEAD: `--adopt {head}`; commits before it must be logged on a card first"
                )),
            CommitRefused::Merge { parents } => Refusal::new("commit", "merge")
                .field("parents", parents)
                .hint("a merge commit cannot be adopted: report the stage blocked"),
            CommitRefused::Adopted { card } => Refusal::new("commit", "adopted")
                .field("card", card)
                .hint(format!(
                    "HEAD is already logged as commit= on {card}: nothing to adopt, report the stage's status"
                )),
            CommitRefused::Unlogged { sha } => Refusal::new("commit", "unlogged")
                .field("sha", sha)
                .hint(format!(
                    "commit {sha} before HEAD is logged on no card of the run: report the stage blocked"
                )),
            CommitRefused::NoMergeBase => Refusal::new("commit", "no-merge-base").hint(
                "the branch point of HEAD with the main branch cannot be found: report the stage blocked",
            ),
            CommitRefused::Files { path } => Refusal::new("commit", "files")
                .field("path", path)
                .hint(format!(
                    "{path} is outside the card's files list: stop for a re-plan"
                )),
            CommitRefused::Trailers => Refusal::new("commit", "trailers").hint(
                "the message carries attribution trailers: amend it with `git commit --amend` if it is not pushed, then adopt again",
            ),
        }
    }
}

/// A commit made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// Its short sha.
    pub sha: String,
    /// The card `commit=` was logged on, or why it could not be.
    pub logged: Option<Result<String, String>>,
}

/// Commits the staged changes in `here` with `message` (attribution
/// lines stripped). With `card`, the card must be a stage in review on a
/// board and HEAD the run's branch; `commit=<sha>` is then logged on it.
///
/// # Errors
/// The board could not be read.
pub fn commit(
    db: Option<&mut Board>,
    here: &Path,
    main: &str,
    card: Option<&str>,
    message: &str,
    now: &str,
) -> rusqlite::Result<Result<Committed, CommitRefused>> {
    let branch = git::out(here, &["symbolic-ref", "--short", "-q", "HEAD"]);
    if branch.as_deref() == Some(main) {
        return Ok(Err(CommitRefused::Main(main.to_owned())));
    }
    let mut on = None;
    if let Some(id) = card {
        if rebasing(here) {
            return Ok(Err(CommitRefused::Rebasing));
        }
        match stage_checks(db, id, branch.as_deref(), false)? {
            Ok(pair) => on = Some(pair),
            Err(r) => return Ok(Err(r)),
        }
    }
    if git::ok(here, &["diff", "--cached", "--quiet"]) && git::commit(here, "MERGE_HEAD").is_none()
    {
        let head = git::out(here, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
        let dirty = git::out(here, &["rev-parse", "--show-toplevel"])
            .is_none_or(|top| gauntlet::dirty(Path::new(&top)));
        return Ok(Err(CommitRefused::NothingStaged { head, dirty }));
    }
    let text = attribution::strip(message).unwrap_or_else(|| message.to_owned());
    let dir = here.to_string_lossy();
    let args = ["-C", &dir, "commit", "-q", "-F", "-"];
    if crate::remote::run("git", &args, Some(&format!("{text}\n")), Path::new("")).is_err() {
        return Ok(Err(CommitRefused::Git));
    }
    let sha = git::out(here, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let logged = on.map(|(db, id)| log_commit(db, id, &sha, now));
    Ok(Ok(Committed { sha, logged }))
}

/// Whether a rebase is stopped in the checkout `here` belongs to.
fn rebasing(here: &Path) -> bool {
    let top = git::out(here, &["rev-parse", "--show-toplevel"]);
    git::rebase_in_progress(top.as_deref().map_or(here, Path::new))
}

fn log_commit(db: &mut Board, id: &str, sha: &str, now: &str) -> Result<String, String> {
    db.write(|w| w.log(id, &format!("commit={sha}"), &[], now))
        .map(|()| id.to_owned())
        .map_err(|e: board::LogError| e.to_string())
}

/// The card and its state and, when asked or when the run names one, its branch.
fn stage_checks<'a>(
    db: Option<&'a mut Board>,
    card: &'a str,
    branch: Option<&str>,
    need_branch: bool,
) -> rusqlite::Result<Result<(&'a mut Board, &'a str), CommitRefused>> {
    let Some(db) = db else {
        return Ok(Err(CommitRefused::Card));
    };
    let run = CardId::parse(card).run;
    let (state, want) =
        db.read(|r| Ok::<_, rusqlite::Error>((r.state(card)?, r.field(run, "branch")?)))?;
    match state {
        None => return Ok(Err(CommitRefused::Card)),
        Some(State::InReview) => {}
        Some(other) => return Ok(Err(CommitRefused::State(other))),
    }
    match want.filter(|b| !b.is_empty()) {
        Some(want) if branch != Some(want.as_str()) => {
            return Ok(Err(CommitRefused::Branch(want)));
        }
        None if need_branch => return Ok(Err(CommitRefused::Branch(String::new()))),
        _ => {}
    }
    Ok(Ok((db, card)))
}

/// Logs `sha`, HEAD of the run's branch committed outside ferrule, as
/// `card`'s `commit=` after the `--adopt` checks; creates no commit.
///
/// # Errors
/// The board could not be read.
pub fn adopt(
    db: Option<&mut Board>,
    here: &Path,
    main: &str,
    card: &str,
    sha: &str,
    now: &str,
) -> rusqlite::Result<Result<Committed, CommitRefused>> {
    let branch = git::out(here, &["symbolic-ref", "--short", "-q", "HEAD"]);
    if branch.as_deref() == Some(main) {
        return Ok(Err(CommitRefused::Main(main.to_owned())));
    }
    if rebasing(here) {
        return Ok(Err(CommitRefused::Rebasing));
    }
    let (db, id) = match stage_checks(db, card, branch.as_deref(), true)? {
        Ok(pair) => pair,
        Err(r) => return Ok(Err(r)),
    };
    if git::out(here, &["rev-parse", "--show-toplevel"])
        .is_none_or(|top| gauntlet::dirty(Path::new(&top)))
    {
        return Ok(Err(CommitRefused::Dirty));
    }
    let head = git::commit(here, "HEAD").unwrap_or_default();
    let short = git::out(here, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    if git::commit(here, sha).is_none_or(|full| full != head) {
        return Ok(Err(CommitRefused::NotHead { head: short }));
    }
    let parents = git::out(here, &["rev-list", "--parents", "-n1", "HEAD"])
        .map_or(0, |l| l.split_whitespace().count().saturating_sub(1));
    if parents != 1 {
        return Ok(Err(CommitRefused::Merge { parents }));
    }
    let run = CardId::parse(id).run;
    let events = db.read(|r| r.run_events(run, &[Name::Commit]))?;
    let mut logged = Vec::new();
    for e in &events {
        if let Some(full) = e.value.as_deref().and_then(|v| git::commit(here, v)) {
            if full == head {
                return Ok(Err(CommitRefused::Adopted {
                    card: e.card.clone(),
                }));
            }
            logged.push(full);
        }
    }
    let range = git::out(here, &["merge-base", "HEAD", main]).map(|base| format!("{base}..HEAD^"));
    let earlier = range.and_then(|r| git::out(here, &["rev-list", "--reverse", &r]));
    let Some(earlier) = earlier else {
        return Ok(Err(CommitRefused::NoMergeBase));
    };
    if let Some(first) = earlier
        .split_whitespace()
        .find(|c| !logged.iter().any(|l| l == c))
    {
        let sha = git::out(here, &["rev-parse", "--short", first]).unwrap_or_default();
        return Ok(Err(CommitRefused::Unlogged { sha }));
    }
    let files = db.read(|r| r.field(id, "files"))?.unwrap_or_default();
    let allowed = board::file_list(&files);
    let touched = git::out(
        here,
        &[
            "diff-tree",
            "-r",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-z",
            "HEAD",
        ],
    )
    .unwrap_or_default();
    if let Some(path) = touched
        .split('\0')
        .find(|p| !p.is_empty() && !allowed.contains(p))
    {
        return Ok(Err(CommitRefused::Files {
            path: path.to_owned(),
        }));
    }
    let message = git::out(here, &["log", "-1", "--format=%B", "HEAD"]).unwrap_or_default();
    if attribution::strip(&message).is_some() {
        return Ok(Err(CommitRefused::Trailers));
    }
    let logged = Some(log_commit(db, id, &short, now));
    Ok(Ok(Committed { sha: short, logged }))
}
