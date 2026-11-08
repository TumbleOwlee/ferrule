//! `run rebase`: the run's branch rebased onto its base, the stage commits
//! the cards logged mapped onto the rebased ones, and the rebase recorded
//! on the run card.

use std::path::{Path, PathBuf};

use crate::card::board::{self, LogError, State};
use crate::card::typed::Name;
use crate::config::Config;
use crate::git;
use crate::project::Layout;
use crate::refusal::Refusal;
use crate::run::{Error, gauntlet};
use crate::store::Board;

/// What a rebase did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rebased {
    /// Mapped: the rebased HEAD (short), the number of `commit=` lines appended.
    Done { head: String, mapped: usize },
    /// The newest logged commit is already on the branch: nothing logged.
    Unchanged { head: String },
    /// Git stopped on a conflict; the rebase stays in progress.
    Conflict { paths: usize },
}

impl std::fmt::Display for Rebased {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rebased::Done { head, mapped } => write!(f, "rebase=done head={head} mapped={mapped}"),
            Rebased::Unchanged { head } => write!(f, "rebase=unchanged head={head}"),
            Rebased::Conflict { paths } => write!(
                f,
                "rebase=conflict paths={paths} hint=resolve the conflicts, stage them with git add or git rm, run GIT_EDITOR=true git rebase --continue, then run the same ferrule run rebase again"
            ),
        }
    }
}

/// Why nothing was rebased or mapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseRefused {
    Card,
    /// HEAD of the worktree is not the run's branch, named.
    Branch(String),
    NoWorktree,
    Dirty,
    /// This stage card is `inprogress`.
    State {
        card: String,
    },
    InProgress,
    NoMergeBase,
    Count {
        old: usize,
        new: usize,
    },
    /// First differing position (1-based) and the two short shas.
    Mismatch {
        at: usize,
        old: String,
        new: String,
    },
    Git(git::GitError),
}

impl RebaseRefused {
    /// The `rebase=refused` line of run `slug`.
    #[must_use]
    pub fn refusal(&self, slug: &str) -> Refusal {
        match self {
            RebaseRefused::Card => Refusal::card("rebase"),
            RebaseRefused::Branch(want) => Refusal::new("rebase", "branch")
                .field("want", want)
                .hint(format!(
                    "check out {want}, the run's branch, in the run's worktree"
                )),
            RebaseRefused::NoWorktree => Refusal::new("rebase", "no-worktree")
                .hint("the run's worktree is missing: `ferrule run reconcile` names it"),
            RebaseRefused::Dirty => Refusal::new("rebase", "dirty").hint(
                "the worktree holds changes: commit them through `ferrule stage commit` or discard them",
            ),
            RebaseRefused::State { card } => Refusal::new("rebase", "state")
                .field("card", card)
                .hint(format!(
                    "wait for {card} to reach inreview, then rebase again"
                )),
            RebaseRefused::InProgress => Refusal::new("rebase", "in-progress").hint(format!(
                "a rebase is already in progress: resolve the conflicts, stage them and run `GIT_EDITOR=true git rebase --continue`, or run `git rebase --abort`, then run `ferrule run rebase {slug}` again"
            )),
            RebaseRefused::NoMergeBase => Refusal::new("rebase", "no-merge-base").hint(
                "the branch point with the base cannot be found: ask the user",
            ),
            RebaseRefused::Count { old, new } => Refusal::new("rebase", "count")
                .field("old", old)
                .field("new", new)
                .hint(format!(
                    "the rebase dropped, squashed or added a commit: ask the user; `git reset --hard` to the newest commit= on the run's cards (`ferrule board show {slug}`) restores the branch"
                )),
            RebaseRefused::Mismatch { at, old, new } => Refusal::new("rebase", "mismatch")
                .field("at", at)
                .field("old", old)
                .field("new", new)
                .hint(format!(
                    "the rebase reordered or reworded a stage commit: ask the user; `git reset --hard` to the newest commit= on the run's cards (`ferrule board show {slug}`) restores the branch"
                )),
            RebaseRefused::Git(e) => Refusal::new("rebase", "git")
                .detail(&e.stderr)
                .hint("git failed: read its message, fix the cause, then run the rebase again"),
        }
    }
}

fn short(wt: &Path, rev: &str) -> String {
    git::out(wt, &["rev-parse", "--short", rev]).unwrap_or_default()
}

fn list(wt: &Path, tip: &str, base: &str) -> Option<Vec<String>> {
    let point = git::out(wt, &["merge-base", tip, base]).filter(|s| !s.is_empty())?;
    let out = git::out(wt, &["rev-list", "--reverse", &format!("{point}..{tip}")])?;
    Some(out.split_whitespace().map(str::to_owned).collect())
}

fn message(wt: &Path, rev: &str) -> String {
    git::out(wt, &["log", "-1", "--format=%B", rev]).unwrap_or_default()
}

/// Each logged `commit=` that sits in `old`, with the short sha of the
/// commit at its position in `new`, once per card and commit.
fn pairs(
    wt: &Path,
    events: &[crate::card::board::Event],
    old: &[String],
    new: &[String],
) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for e in events {
        let Some(full) = e.value.as_deref().and_then(|v| git::commit(wt, v)) else {
            continue;
        };
        let Some(i) = old.iter().position(|o| *o == full) else {
            continue;
        };
        let pair = (e.card.clone(), short(wt, &new[i]));
        if !pairs.contains(&pair) {
            pairs.push(pair);
        }
    }
    pairs
}

/// Rebases run `slug`'s branch onto `onto` (default `origin/<main branch>`,
/// fetched first), then logs each stage commit's rebased twin on its card
/// and `rebase=` on the run card.
///
/// # Errors
/// The board could not be read or written.
pub fn rebase(
    db: &mut Board,
    layout: &Layout,
    config: &Config,
    slug: &str,
    onto: Option<&str>,
    now: &str,
) -> Result<Result<Rebased, RebaseRefused>, Error> {
    let (state, wt, branch, stages, events) = db.read(|r| {
        Ok::<_, rusqlite::Error>((
            r.state(slug)?,
            r.worktree(layout, slug, slug)?,
            r.field(slug, "branch")?,
            board::stages(r.conn(), slug)?,
            r.run_events(slug, &[Name::Commit])?,
        ))
    })?;
    if state.is_none() {
        return Ok(Err(RebaseRefused::Card));
    }
    if !wt.is_dir() {
        return Ok(Err(RebaseRefused::NoWorktree));
    }
    if git::rebase_in_progress(&wt) {
        return Ok(Err(RebaseRefused::InProgress));
    }
    let branch = branch.unwrap_or_default();
    if git::out(&wt, &["symbolic-ref", "--short", "-q", "HEAD"]).as_deref() != Some(&branch) {
        return Ok(Err(RebaseRefused::Branch(branch)));
    }
    if gauntlet::dirty(&wt) {
        return Ok(Err(RebaseRefused::Dirty));
    }
    if let Some((_, id, _)) = stages.iter().find(|(_, _, s)| *s == State::InProgress) {
        return Ok(Err(RebaseRefused::State { card: id.clone() }));
    }
    let base = if let Some(b) = onto {
        b.to_owned()
    } else {
        if let Err(e) = git::run(&wt, &["fetch", "-q", "origin", &config.main_branch]) {
            return Ok(Err(RebaseRefused::Git(e)));
        }
        format!("origin/{}", config.main_branch)
    };
    if let Err(e) = git::run(&wt, &["rebase", "--no-autosquash", &base]) {
        if git::rebase_in_progress(&wt) {
            let paths = git::out(&wt, &["diff", "--name-only", "--diff-filter=U"])
                .map_or(0, |s| s.lines().count());
            return Ok(Ok(Rebased::Conflict { paths }));
        }
        return Ok(Err(RebaseRefused::Git(e)));
    }
    let head = short(&wt, "HEAD");
    let Some(newest) = events.last().and_then(|e| e.value.as_deref()) else {
        return Ok(Ok(Rebased::Unchanged { head }));
    };
    let Some(old_head) = git::commit(&wt, newest) else {
        return Ok(Err(RebaseRefused::NoMergeBase));
    };
    if git::is_ancestor(&wt, &old_head, "HEAD") {
        return Ok(Ok(Rebased::Unchanged { head }));
    }
    let (Some(old), Some(new)) = (list(&wt, &old_head, &base), list(&wt, "HEAD", &base)) else {
        return Ok(Err(RebaseRefused::NoMergeBase));
    };
    if old.len() != new.len() {
        return Ok(Err(RebaseRefused::Count {
            old: old.len(),
            new: new.len(),
        }));
    }
    for (i, (o, n)) in old.iter().zip(&new).enumerate() {
        if message(&wt, o) != message(&wt, n) {
            return Ok(Err(RebaseRefused::Mismatch {
                at: i + 1,
                old: short(&wt, o),
                new: short(&wt, n),
            }));
        }
    }
    let pairs = pairs(&wt, &events, &old, &new);
    let from = short(&wt, &old_head);
    db.write(|w| {
        for (card, sha) in &pairs {
            w.log(card, &format!("commit={sha}"), &[], now)?;
        }
        w.log(
            slug,
            &format!("rebase={head}"),
            &[format!("base={base}"), format!("from={from}")],
            now,
        )
    })
    .map_err(|e: LogError| match e {
        LogError::Sql(e) => Error::Sql(e),
        _ => Error::Card,
    })?;
    Ok(Ok(Rebased::Done {
        head,
        mapped: pairs.len(),
    }))
}

/// The run worktrees under `.ferrule/worktrees/` with a rebase in progress.
#[must_use]
pub fn rebasing_worktrees(layout: &Layout) -> Vec<PathBuf> {
    let Some(parent) = layout.worktree("x").parent().map(Path::to_path_buf) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && git::rebase_in_progress(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_rebased_prints_its_line() {
        let done = Rebased::Done {
            head: "abc".into(),
            mapped: 3,
        };
        assert_eq!(done.to_string(), "rebase=done head=abc mapped=3");
        let same = Rebased::Unchanged { head: "abc".into() };
        assert_eq!(same.to_string(), "rebase=unchanged head=abc");
        assert!(
            Rebased::Conflict { paths: 2 }
                .to_string()
                .starts_with("rebase=conflict paths=2 hint=")
        );
    }

    #[test]
    fn ut_refusal_names_its_reason_and_fields() {
        let line = |r: RebaseRefused| r.refusal("demo").to_string();
        assert!(line(RebaseRefused::Dirty).starts_with("rebase=refused reason=dirty hint="));
        assert!(
            line(RebaseRefused::Count { old: 2, new: 1 })
                .starts_with("rebase=refused reason=count old=2 new=1 hint=")
        );
        assert!(
            line(RebaseRefused::Mismatch {
                at: 2,
                old: "a".into(),
                new: "b".into()
            })
            .starts_with("rebase=refused reason=mismatch at=2 old=a new=b hint=")
        );
        assert!(line(RebaseRefused::InProgress).contains("ferrule run rebase demo"));
    }
}
