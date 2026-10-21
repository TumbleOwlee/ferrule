//! The run commands that answer from the board and git alone: verifying a
//! reported commit, reconciling cards with git, and the stage-stop
//! decision.

pub mod archive;
pub mod ci;
pub mod commit;
pub mod gate;
pub mod gauntlet;
pub mod ids;
pub mod inspect;
pub mod next;
pub mod push;
pub mod rebuild;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::card::board::{self, State};
use crate::card::typed::{Name, Typed};
use crate::config::Config;
use crate::project::Layout;
use crate::store::{Board, Reader};
use crate::{artifacts, git};

/// A run command's failure that is not an answer.
#[derive(Debug)]
pub enum Error {
    /// The run or stage has no card.
    Card,
    Sql(rusqlite::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Card => f.write_str("the run or stage has no card"),
            Error::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Card => None,
            Error::Sql(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

pub use crate::card::board::worktree;

impl Reader<'_> {
    /// [`worktree`] over this snapshot.
    pub fn worktree(&self, layout: &Layout, slug: &str, card: &str) -> rusqlite::Result<PathBuf> {
        worktree(self.conn(), layout, slug, card)
    }
}

/// Why a reported commit is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    Card,
    Missing,
    OffBranch,
    CardCommit,
}

impl Mismatch {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Mismatch::Card => "card",
            Mismatch::Missing => "missing",
            Mismatch::OffBranch => "off-branch",
            Mismatch::CardCommit => "card-commit",
        }
    }
}

/// Makes `wt` a worktree on `branch`: reused when it already is one, else
/// added after a prune, on the existing `branch` or a new one cut from
/// `start`.
///
/// # Errors
/// [`git::GitError`] when git cannot add it.
pub fn ensure_worktree(
    layout: &Layout,
    wt: &Path,
    branch: &str,
    start: &str,
) -> Result<(), git::GitError> {
    let root = &layout.root;
    if wt.is_dir()
        && git::out(wt, &["rev-parse", "--show-toplevel"])
            .is_some_and(|top| same_dir(Path::new(&top), wt))
        && git::out(wt, &["symbolic-ref", "--short", "-q", "HEAD"]).as_deref() == Some(branch)
    {
        return Ok(());
    }
    let _ = git::ok(root, &["worktree", "prune"]);
    let wt = wt.as_os_str();
    let branch_ref = format!("refs/heads/{branch}");
    if git::ok(root, &["rev-parse", "--verify", "-q", &branch_ref]) {
        let add = [
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-q"),
            wt,
            OsStr::new(branch),
        ];
        git::run(root, &add)?;
    } else {
        let add = [
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-q"),
            OsStr::new("-b"),
            OsStr::new(branch),
            wt,
            OsStr::new(start),
        ];
        git::run(root, &add)?;
    }
    Ok(())
}

/// Checks an implementer's reported commit: it exists in the worktree, the
/// run's branch contains it, and it is the stage card's last `commit=`.
pub fn sha_check(
    r: &Reader<'_>,
    layout: &Layout,
    slug: &str,
    sid: &str,
    sha: &str,
) -> rusqlite::Result<Result<(), Mismatch>> {
    let conn = r.conn();
    let card = format!("{slug}.{sid}");
    if board::state(conn, slug)?.is_none() || board::state(conn, &card)?.is_none() {
        return Ok(Err(Mismatch::Card));
    }
    let wt = worktree(conn, layout, slug, &card)?;
    let Some(full) = git::commit(&wt, sha) else {
        return Ok(Err(Mismatch::Missing));
    };
    let branch = board::field(conn, slug, "branch")?.unwrap_or_default();
    let on_branch = !branch.is_empty()
        && git::out(&wt, &["branch", "--contains", &full]).is_some_and(|out| {
            out.lines()
                .any(|l| l.get(2..).is_some_and(|name| name == branch))
        });
    if !on_branch {
        return Ok(Err(Mismatch::OffBranch));
    }
    let logged = board::last_value(conn, &card, Name::Commit)?;
    if logged.and_then(|l| git::commit(&wt, &l)).as_deref() != Some(full.as_str()) {
        return Ok(Err(Mismatch::CardCommit));
    }
    Ok(Ok(()))
}

/// Disagreements between a run's cards and git, and the first stage not
/// done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// `<row> <detail>` per disagreement.
    pub differ: Vec<String>,
    /// `s<n>`, or `None` when every stage is done.
    pub next: Option<String>,
}

pub(crate) fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Card = intent, git = fact: the worktree once `gate2` is set, the branch,
/// every stage's `commit=`, every done stage having one, and the last
/// `gauntlet=pass` sha on the branch.
pub fn reconcile(r: &Reader<'_>, layout: &Layout, slug: &str) -> Result<Reconciled, Error> {
    let conn = r.conn();
    if board::state(conn, slug)?.is_none() {
        return Err(Error::Card);
    }
    let root = &layout.root;
    let mut differ = Vec::new();
    let branch = board::field(conn, slug, "branch")?.filter(|b| !b.is_empty());
    let wt = worktree(conn, layout, slug, slug)?;
    if board::field(conn, slug, "gate2")?.is_some_and(|g| !g.is_empty()) {
        let registered = git::out(root, &["worktree", "list", "--porcelain"]).is_some_and(|out| {
            out.lines()
                .filter_map(|l| l.strip_prefix("worktree "))
                .any(|p| same_dir(Path::new(p), &wt))
        });
        if !registered || !wt.is_dir() {
            differ.push(format!("worktree {} missing", wt.display()));
        }
    }
    let branch_ref = branch.as_ref().map(|b| format!("refs/heads/{b}"));
    let on_branch = match (&branch, &branch_ref) {
        (Some(b), Some(r)) => {
            let exists = git::commit(root, r).is_some();
            if !exists {
                differ.push(format!("branch {b} missing"));
            }
            exists
        }
        _ => false,
    };
    let contains = |sha: &str| {
        on_branch
            && branch_ref
                .as_ref()
                .is_some_and(|r| git::is_ancestor(root, sha, r))
    };
    let mut next = None;
    for (n, id, state) in board::stages(conn, slug)? {
        match board::last_value(conn, &id, Name::Commit)? {
            Some(sha) => {
                if git::commit(root, &sha).is_none() {
                    differ.push(format!("commit s{n} {sha} does not exist"));
                } else if let Some(b) = &branch
                    && !contains(&sha)
                {
                    differ.push(format!("commit s{n} {sha} not on {b}"));
                }
            }
            None if state == State::Done => differ.push(format!("done s{n} has no commit")),
            None => {}
        }
        if state != State::Done && next.is_none() {
            next = Some(format!("s{n}"));
        }
    }
    if let (Some(g), Some(b)) = (board::pass_shas(conn, slug)?.pop(), &branch)
        && !contains(&g)
    {
        differ.push(format!("gauntlet {g} not on {b}"));
    }
    Ok(Reconciled { differ, next })
}

/// Why a stage stops for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    NeedsUser,
    Verification(String),
    PrThreads(u32),
    PlanMarked,
    DiffSize { lines: usize, files: usize },
    Checkpoint,
}

impl Stop {
    /// The rule number logged as `stop=<n>`.
    #[must_use]
    pub fn rule(&self) -> u8 {
        match self {
            Stop::NeedsUser => 1,
            Stop::Verification(_) => 2,
            Stop::PrThreads(_) => 3,
            Stop::PlanMarked => 5,
            Stop::DiffSize { .. } => 6,
            Stop::Checkpoint => 7,
        }
    }
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.rule();
        match self {
            Stop::NeedsUser => write!(f, "stop={n} reason=needs-user"),
            Stop::Verification(what) => write!(f, "stop={n} reason=verification detail=\"{what}\""),
            Stop::PrThreads(t) => write!(f, "stop={n} reason=pr-threads detail=\"{t} threads\""),
            Stop::PlanMarked => write!(f, "stop={n} reason=plan-marked"),
            Stop::DiffSize { lines, files } => write!(
                f,
                "stop={n} reason=diff-size detail=\"{lines} lines, {files} files\""
            ),
            Stop::Checkpoint => write!(f, "stop={n} reason=checkpoint"),
        }
    }
}

/// Changed lines and files in `wt` since `base`, untracked files included.
fn diff_size(wt: &Path, base: &str) -> Option<(usize, usize)> {
    let numstat = git::out(wt, &["diff", "--numstat", base])?;
    let (mut lines, mut files) = (0, 0);
    for l in numstat.lines().filter(|l| !l.is_empty()) {
        let mut cols = l.split('\t');
        let add: usize = cols.next().and_then(|c| c.parse().ok()).unwrap_or(0);
        let del: usize = cols.next().and_then(|c| c.parse().ok()).unwrap_or(0);
        lines += add + del;
        files += 1;
    }
    let untracked =
        git::out(wt, &["ls-files", "-z", "--others", "--exclude-standard"]).unwrap_or_default();
    // `-z`: git quotes non-ASCII paths otherwise.
    for u in untracked.split('\0').filter(|l| !l.is_empty()) {
        // `wc -l`: newlines, not lines.
        lines += std::fs::read(wt.join(u)).map_or(0, |b| b.split(|&c| c == b'\n').count() - 1);
        files += 1;
    }
    Some((lines, files))
}

/// Whether a green, clean-reviewed full-lane stage stops for approval;
/// `None` means it auto-continues. s0 never stops.
fn decide(
    conn: &Connection,
    layout: &Layout,
    config: &Config,
    slug: &str,
    me: u32,
    threads: u32,
) -> rusqlite::Result<Option<Stop>> {
    if me == 0 {
        return Ok(None);
    }
    let sid = format!("s{me}");
    let card = format!("{slug}.{sid}");
    if layout
        .artifact(slug, "review.verdict.md")
        .is_some_and(|v| artifacts::needs_user(&v))
    {
        return Ok(Some(Stop::NeedsUser));
    }
    if let Some(what) = layout
        .artifact(slug, "plan.summary.md")
        .and_then(|s| artifacts::verification(&s, &sid))
    {
        return Ok(Some(Stop::Verification(what)));
    }
    if threads > 0 {
        return Ok(Some(Stop::PrThreads(threads)));
    }
    if board::field(conn, &card, "stop")?.as_deref() == Some("yes") {
        return Ok(Some(Stop::PlanMarked));
    }
    let stages = board::stages(conn, slug)?;
    let wt = worktree(conn, layout, slug, &card)?;
    let mut base = None;
    for (_, id, _) in stages.iter().rev().filter(|(n, _, _)| *n < me) {
        base = board::last_value(conn, id, Name::Commit)?;
        if base.is_some() {
            break;
        }
    }
    let base = base.or_else(|| git::out(&wt, &["merge-base", "HEAD", &config.main_branch]));
    if let Some(base) = base.filter(|b| git::commit(&wt, b).is_some())
        && let Some((lines, files)) = diff_size(&wt, &base)
        && (lines > config.thresholds.stage_stop_lines
            || files > config.thresholds.stage_stop_files)
    {
        return Ok(Some(Stop::DiffSize { lines, files }));
    }
    let mut run = 0;
    for (_, id, _) in stages.iter().filter(|(n, _, _)| (1..me).contains(n)) {
        match board::last_event(conn, id, &[Name::Auto, Name::StageStop])? {
            Some(e) if e.typed() == Typed::Auto => run += 1,
            Some(_) => run = 0,
            None => {}
        }
    }
    Ok((run >= config.thresholds.stage_stop_checkpoint).then_some(Stop::Checkpoint))
}

/// The stage-stop decision for `s<n>`, logged on its card as `stop=<rule>`
/// or `auto=yes`.
pub fn stage_stop(
    db: &mut Board,
    layout: &Layout,
    config: &Config,
    slug: &str,
    me: u32,
    threads: u32,
    now: &str,
) -> Result<Option<Stop>, Error> {
    let conn = db.conn_mut();
    let card = format!("{slug}.s{me}");
    if board::state(conn, &card)?.is_none() {
        return Err(Error::Card);
    }
    let stop = decide(conn, layout, config, slug, me, threads)?;
    let event = match &stop {
        Some(s) => format!("stop={}", s.rule()),
        None => "auto=yes".to_owned(),
    };
    match board::log(conn, &card, &event, &[], now) {
        Ok(()) => Ok(stop),
        Err(board::LogError::Sql(e)) => Err(Error::Sql(e)),
        Err(board::LogError::Card | board::LogError::Reject(_)) => Err(Error::Card),
    }
}
