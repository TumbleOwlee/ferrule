//! Abandoning a run the user gives up: close its PR, delete its branches,
//! archive its cards as abandoned and remove its worktree.

use std::path::PathBuf;

use rusqlite::Connection;

use super::archive::{self, CiCheck, CleanupRefusal, Seal};
use super::diverged::{Remote, remote_head};
use crate::card::board;
use crate::git;
use crate::project::Layout;
use crate::refusal::Refusal;
use crate::remote::{self, CodeHost, ItemRef, PrState, refs};
use crate::store::{Board, Reader};

const ORIGIN: &str = "origin";

/// The comment left on the PR an abandon closes.
pub const CLOSE_COMMENT: &str = "Run abandoned: this PR is closed unmerged. The issue stays open.";

/// What a confirmed abandon would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbandonPlan {
    /// Paths `git status --porcelain` lists in the worktree; empty without one.
    pub uncommitted: Vec<String>,
    /// `<short-sha> <subject>` of the local branch's commits on no origin branch.
    pub unpushed: Vec<String>,
    /// The open PR to comment on and close; `None` without one or when closed.
    pub pr: Option<ItemRef>,
    /// The branch on origin to delete; `None` when origin lacks it.
    pub remote_branch: Option<String>,
    pub local_branch: Option<String>,
    pub worktree: Option<PathBuf>,
    /// The abandoned archive when the cards are already archived (rerun).
    pub archived: Option<i64>,
}

impl AbandonPlan {
    fn empty() -> Self {
        AbandonPlan {
            uncommitted: Vec::new(),
            unpushed: Vec::new(),
            pr: None,
            remote_branch: None,
            local_branch: None,
            worktree: None,
            archived: None,
        }
    }
}

/// What an abandon did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abandoned {
    pub archive: Option<i64>,
    /// The issue the run references, left open.
    pub issue: Option<ItemRef>,
    /// An earlier abandon archived the run; this one finished its local steps.
    pub resumed: bool,
    /// The run was already fully abandoned; nothing changed.
    pub already: bool,
}

#[derive(Debug)]
pub enum AbandonRefused {
    /// No live card and no abandoned archive.
    Card,
    /// The run is an epic's parent.
    Epic,
    /// The PR is merged; the hint names `ferrule run cleanup`.
    Merged(ItemRef),
    /// The PR's state could not be read.
    PrUnreadable(remote::Error),
    /// The remote branch could not be looked up.
    RemoteUnreadable(String),
    ClosePr(remote::Error),
    DeleteRemote(git::GitError),
    /// Worktree or local branch removal failed after the archive.
    Local(CleanupRefusal),
}

impl AbandonRefused {
    /// The reason word.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            AbandonRefused::Card => "card",
            AbandonRefused::Epic => "epic",
            AbandonRefused::Merged(_) => "merged",
            AbandonRefused::PrUnreadable(_) => "pr-unreadable",
            AbandonRefused::RemoteUnreadable(_) => "remote-unreadable",
            AbandonRefused::ClosePr(_) => "close-pr-failed",
            AbandonRefused::DeleteRemote(_) => "remote-delete-failed",
            AbandonRefused::Local(r) => r.as_str(),
        }
    }

    /// The `abandon=refused` line of run `slug`.
    #[must_use]
    pub fn refusal(&self, slug: &str) -> Refusal {
        if matches!(self, AbandonRefused::Card) {
            return Refusal::card("abandon");
        }
        let r = Refusal::new("abandon", self.as_str());
        match self {
            AbandonRefused::Card => r,
            AbandonRefused::Epic => r.hint(format!(
                "abandon its slice runs one by one; the epic ends with ferrule run cleanup {slug}"
            )),
            AbandonRefused::Merged(pr) => r.field("pr", pr).hint(format!(
                "the PR is merged: end the run with ferrule run cleanup {slug}"
            )),
            AbandonRefused::PrUnreadable(e) => r
                .detail(e.to_string())
                .hint("check the host login, then rerun"),
            AbandonRefused::RemoteUnreadable(b) => r
                .field("branch", b)
                .hint("check that origin is reachable, then rerun"),
            AbandonRefused::ClosePr(e) => r
                .detail(e.to_string())
                .hint("close the PR on the host by hand, then rerun"),
            AbandonRefused::DeleteRemote(e) => r
                .detail(&e.stderr)
                .hint("ask the user to delete the branch on origin, then rerun"),
            AbandonRefused::Local(
                CleanupRefusal::WorktreeRemoveFailed(e) | CleanupRefusal::BranchDeleteFailed(e),
            ) => r.detail(&e.stderr).hint(format!(
                "ask the user to remove it, then rerun ferrule run abandon {slug} --confirm"
            )),
            AbandonRefused::Local(_) => {
                r.hint(format!("rerun ferrule run abandon {slug} --confirm"))
            }
        }
    }
}

fn branch_exists(root: &std::path::Path, branch: &str) -> bool {
    git::ok(
        root,
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{branch}"),
        ],
    )
}

/// What a confirmed abandon would do; changes nothing. Refuses as `abandon`
/// does.
///
/// A worktree with a rebase in progress is planned as any other: its
/// conflicted and modified files already show as uncommitted, and the
/// removal is forced.
pub fn plan(
    r: &Reader<'_>,
    layout: &Layout,
    slug: &str,
    host: &dyn CodeHost,
) -> Result<Result<AbandonPlan, AbandonRefused>, archive::Error> {
    let conn = r.conn();
    let mut plan = AbandonPlan::empty();
    if board::state(conn, slug)?.is_none() {
        let Some(id) = archive::last_abandoned(conn, slug)? else {
            return Ok(Err(AbandonRefused::Card));
        };
        plan.archived = Some(id);
        if let Some(todo) = archive::pending(conn, slug)? {
            plan.worktree = todo.worktree.map(PathBuf::from).filter(|p| p.is_dir());
            plan.local_branch = todo.branch.filter(|b| branch_exists(&layout.root, b));
        }
        return Ok(Ok(plan));
    }
    if board::field(conn, slug, "kind")?.as_deref() == Some("epic") {
        return Ok(Err(AbandonRefused::Epic));
    }
    let pr_field = board::field(conn, slug, "pr")?;
    if let Some(pr) = refs(pr_field.as_deref()).into_iter().next() {
        match host.status(&pr) {
            Err(e) => return Ok(Err(AbandonRefused::PrUnreadable(e))),
            Ok(st) => match st.state {
                PrState::Merged => return Ok(Err(AbandonRefused::Merged(pr))),
                PrState::Open => plan.pr = Some(pr),
                PrState::Closed => {}
            },
        }
    }
    let branch = board::field(conn, slug, "branch")?.filter(|b| !b.is_empty());
    if let Some(b) = branch {
        match remote_head(&layout.root, &b) {
            None => return Ok(Err(AbandonRefused::RemoteUnreadable(b))),
            Some(Remote::At(_)) => plan.remote_branch = Some(b.clone()),
            Some(Remote::Missing) => {}
        }
        if branch_exists(&layout.root, &b) {
            let range = format!("refs/heads/{b}");
            let remotes = format!("--remotes={ORIGIN}");
            plan.unpushed = git::out(
                &layout.root,
                &["log", "--format=%h %s", &range, "--not", &remotes],
            )
            .map(|o| o.lines().map(str::to_owned).collect())
            .unwrap_or_default();
            plan.local_branch = Some(b);
        }
    }
    let wt = super::worktree(conn, layout, slug, slug)?;
    if wt.is_dir() {
        plan.uncommitted = git::out(&wt, &["status", "--porcelain"])
            .map(|o| {
                o.lines()
                    .map(|l| l.get(3..).unwrap_or(l).to_owned())
                    .collect()
            })
            .unwrap_or_default();
        plan.worktree = Some(wt);
    }
    Ok(Ok(plan))
}

fn local(e: archive::Error) -> Result<AbandonRefused, archive::Error> {
    match e {
        archive::Error::Refused(r) => Ok(AbandonRefused::Local(r)),
        other => Err(other),
    }
}

/// Comments on and closes the PR, deletes the remote branch, archives as
/// abandoned, removes the local state; each step only when still missing.
/// The run's issue is never read or written.
pub fn abandon(
    db: &mut Board,
    layout: &Layout,
    slug: &str,
    now: &str,
    host: &dyn CodeHost,
) -> Result<Result<Abandoned, AbandonRefused>, archive::Error> {
    let conn: &mut Connection = db.conn_mut();
    let plan = match plan(&Reader::new(conn), layout, slug, host)? {
        Ok(p) => p,
        Err(r) => return Ok(Err(r)),
    };
    if let Some(id) = plan.archived {
        return match archive::resume(conn, layout, slug) {
            Ok(Some(id)) => Ok(Ok(Abandoned {
                archive: Some(id),
                issue: archive::archived_issue(conn, id, slug)?,
                resumed: true,
                already: false,
            })),
            Ok(None) => Ok(Ok(Abandoned {
                archive: Some(id),
                issue: None,
                resumed: false,
                already: true,
            })),
            Err(e) => local(e).map(Err),
        };
    }
    if let Some(pr) = &plan.pr
        && let Err(e) = host.close(pr, CLOSE_COMMENT)
    {
        return Ok(Err(AbandonRefused::ClosePr(e)));
    }
    if let Some(b) = &plan.remote_branch
        && let Err(e) = git::run(&layout.root, &["push", "origin", "--delete", b])
    {
        return Ok(Err(AbandonRefused::DeleteRemote(e)));
    }
    let issue = archive::issue_ref(conn, slug)?;
    let seal = Seal {
        worktree: plan.worktree.map(|p| p.to_string_lossy().into_owned()),
        branch: plan.local_branch,
        check: CiCheck::Exempt,
        abandoned: true,
    };
    match archive::seal(conn, layout, slug, now, seal) {
        Ok(archive) => Ok(Ok(Abandoned {
            archive,
            issue,
            resumed: false,
            already: false,
        })),
        Err(e) => local(e).map(Err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::board::State;
    use crate::remote::fake::FakeHost;

    const NOW: &str = "2026-11-10T12:00";

    fn db(fields: &[(&str, &str)]) -> Board {
        let mut db = Board::memory().unwrap();
        let f: Vec<(String, String)> = fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        db.write(|w| w.create("demo", State::Open, &f, NOW))
            .unwrap();
        db
    }

    fn planned(db: &Board, slug: &str, host: &FakeHost) -> Result<AbandonPlan, AbandonRefused> {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().to_path_buf());
        db.read(|r| plan(r, &layout, slug, host)).unwrap()
    }

    /// RU-E-029, RU-E-024, RU-E-023 — a missing card, an epic and a merged PR
    /// are refused before any git call, the epic without a host call.
    #[test]
    fn ut_plan_refuses_before_any_git_call() {
        let host = FakeHost::default();
        let empty = db(&[]);
        assert!(matches!(
            planned(&empty, "nope", &host),
            Err(AbandonRefused::Card)
        ));
        let epic = db(&[("kind", "epic"), ("pr", "34")]);
        assert!(matches!(
            planned(&epic, "demo", &host),
            Err(AbandonRefused::Epic)
        ));
        assert!(host.calls().is_empty());
        "merged".clone_into(&mut host.0.borrow_mut().state);
        let merged = db(&[("pr", "34")]);
        assert!(matches!(
            planned(&merged, "demo", &host),
            Err(AbandonRefused::Merged(_))
        ));
    }

    /// Every refusal names its reason word.
    #[test]
    fn ut_refusal_reasons() {
        let git = || git::GitError {
            args: Vec::new(),
            code: Some(1),
            stderr: String::new(),
        };
        let err = || remote::Error::NotFound(String::new());
        let pr = ItemRef::from(34);
        let got = [
            AbandonRefused::Card.as_str(),
            AbandonRefused::Epic.as_str(),
            AbandonRefused::Merged(pr).as_str(),
            AbandonRefused::PrUnreadable(err()).as_str(),
            AbandonRefused::RemoteUnreadable(String::new()).as_str(),
            AbandonRefused::ClosePr(err()).as_str(),
            AbandonRefused::DeleteRemote(git()).as_str(),
            AbandonRefused::Local(CleanupRefusal::WorktreeRemoveFailed(git())).as_str(),
            AbandonRefused::Local(CleanupRefusal::BranchDeleteFailed(git())).as_str(),
        ];
        assert_eq!(
            got,
            [
                "card",
                "epic",
                "merged",
                "pr-unreadable",
                "remote-unreadable",
                "close-pr-failed",
                "remote-delete-failed",
                "worktree-remove-failed",
                "branch-delete-failed",
            ]
        );
    }
}
