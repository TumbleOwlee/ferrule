//! The end of a run: its metrics line, the clean-up that removes its
//! worktree and moves its cards and artifacts into a compressed archive,
//! and the export that writes an archive back out as files.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::card::board::{self, Event};
use crate::card::typed::Typed;
use crate::git;
use crate::project::Layout;
use crate::refusal::Refusal;
use crate::store::{Board, Reader, Writer};

/// Why a run's metrics are refused: its CI is not known green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiRefusal {
    NoCi,
    CiRed,
}

impl CiRefusal {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CiRefusal::NoCi => "no-ci",
            CiRefusal::CiRed => "ci-red",
        }
    }

    /// The `<verb>=refused` line of run `slug`.
    #[must_use]
    pub fn refusal(self, verb: &'static str, slug: &str) -> Refusal {
        let r = Refusal::new(verb, self.as_str());
        match self {
            CiRefusal::NoCi => r.hint(format!("record CI first: ferrule ci wait {slug}")),
            CiRefusal::CiRed => r.hint(format!(
                "fix CI and rerun `ferrule ci wait {slug}`; on the user's word only: \
                 ferrule board log {slug} ci-waived -- <why>"
            )),
        }
    }
}

/// Why a clean-up refused before it changed anything.
#[derive(Debug)]
pub enum CleanupRefusal {
    /// The worktree is on no branch.
    DetachedHead,
    /// The worktree has changes.
    UncommittedChanges,
    /// The branch was never pushed.
    NoUpstream,
    /// The branch has commits its upstream lacks.
    UnpushedCommits,
    /// `git worktree remove` failed.
    WorktreeRemoveFailed(git::GitError),
    /// `git branch -D` failed.
    BranchDeleteFailed(git::GitError),
}

impl CleanupRefusal {
    /// The reason word.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            CleanupRefusal::DetachedHead => "detached-head",
            CleanupRefusal::UncommittedChanges => "uncommitted-changes",
            CleanupRefusal::NoUpstream => "no-upstream",
            CleanupRefusal::UnpushedCommits => "unpushed-commits",
            CleanupRefusal::WorktreeRemoveFailed(_) => "worktree-remove-failed",
            CleanupRefusal::BranchDeleteFailed(_) => "branch-delete-failed",
        }
    }

    /// The `cleanup=refused` line of run `slug`.
    #[must_use]
    pub fn refusal(&self, slug: &str) -> Refusal {
        let r = Refusal::new("cleanup", self.as_str());
        match self {
            CleanupRefusal::DetachedHead => r.hint("check the run's branch out in its worktree"),
            CleanupRefusal::UncommittedChanges => {
                r.hint("commit them with `ferrule stage commit`, or ask the user, then push")
            }
            CleanupRefusal::NoUpstream | CleanupRefusal::UnpushedCommits => {
                r.hint(format!("push the branch: ferrule pr push {slug}"))
            }
            CleanupRefusal::WorktreeRemoveFailed(e) => r.detail(&e.stderr).hint(
                "the shell guard refuses `git worktree remove`; ask the user to remove it, then rerun",
            ),
            CleanupRefusal::BranchDeleteFailed(e) => r
                .detail(&e.stderr)
                .hint("delete the branch by hand with `git branch -D`, then rerun"),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Card,
    Ci(CiRefusal),
    /// A clean-up refused before it changed anything.
    Refused(CleanupRefusal),
    Io(std::io::Error),
    Sql(rusqlite::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Card => f.write_str("the run has no card"),
            Error::Ci(r) => write!(f, "CI refused: {}", r.as_str()),
            Error::Refused(r) => write!(f, "refused: {}", r.as_str()),
            Error::Io(e) => write!(f, "{e}"),
            Error::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Sql(e) => Some(e),
            Error::Card | Error::Ci(_) | Error::Refused(_) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Lines that `pick` keeps.
fn count(events: &[Event], pick: impl Fn(Typed) -> bool) -> usize {
    events.iter().filter(|e| pick(e.typed())).count()
}

/// `stop kind=<k>` lines.
fn stops(events: &[Event], kind: &str) -> usize {
    count(
        events,
        |t| matches!(t, Typed::Stop { kind: Some(k), .. } if k == kind),
    )
}

/// The run's last `ci=` value, `waived` when a `ci-waived` line is newer.
fn ci(events: &[Event]) -> Result<String, CiRefusal> {
    let mut last = None;
    let mut waived_after = false;
    for e in events {
        match e.typed() {
            Typed::Ci(v) => {
                last = Some(v.to_owned());
                waived_after = false;
            }
            Typed::CiWaived if last.is_some() => waived_after = true,
            _ => {}
        }
    }
    match last {
        None => Err(CiRefusal::NoCi),
        Some(_) if waived_after => Ok("waived".to_owned()),
        Some(v) if v == "0" => Ok(v),
        Some(_) => Err(CiRefusal::CiRed),
    }
}

/// The run's metrics line, recorded (replacing a line for the same run and
/// start) unless its CI is not known green.
pub fn metrics(w: &Writer<'_>, slug: &str, now: &str) -> Result<String, Error> {
    let conn = w.conn();
    if board::state(conn, slug)?.is_none() {
        return Err(Error::Card);
    }
    let parent = board::events(conn, slug, &[])?;
    let stages = board::stages(conn, slug)?;
    let mut stage_events = Vec::new();
    for (_, id, _) in &stages {
        stage_events.extend(board::events(conn, id, &[])?);
    }
    let start = parent
        .iter()
        .chain(&stage_events)
        .map(|e| e.at.as_str())
        .filter(|at| board::valid_stamp(at))
        .min()
        .unwrap_or("-")
        .to_owned();
    let first_red = stage_events
        .iter()
        .filter(|e| board::valid_stamp(&e.at) && e.typed() == Typed::TestRed)
        .map(|e| e.at.as_str())
        .min()
        .unwrap_or("-");
    let epic = board::field(conn, slug, "kind")?.as_deref() == Some("epic");
    let lane = if epic {
        "epic".to_owned()
    } else {
        board::field(conn, slug, "lane")?.unwrap_or_else(|| "-".to_owned())
    };
    let ci = if epic {
        "-".to_owned()
    } else {
        ci(&parent).map_err(Error::Ci)?
    };
    let overturned: u64 = parent
        .iter()
        .filter_map(|e| match e.typed() {
            Typed::Overturned { count } => count,
            _ => None,
        })
        .sum();
    let reads = count(&stage_events, |t| {
        matches!(t, Typed::Read { path: Some(_) })
    });
    let metrics_line = format!(
        "{slug} start={start} end={now} first-red={first_red} stages={} stops={} question={} approval={} stage={} blocked={} replans={} overturned={overturned} reads={reads} lane={lane} ci={ci}",
        stages.len(),
        count(&parent, |t| matches!(t, Typed::Stop { kind: Some(_), .. })),
        stops(&parent, "question"),
        stops(&parent, "approval"),
        stops(&parent, "stage"),
        stops(&parent, "blocked"),
        count(&parent, |t| matches!(t, Typed::Replan { from: Some(_) })),
    );
    conn.execute(
        "INSERT INTO metrics (run, start, line) VALUES (?1, ?2, ?3)
         ON CONFLICT (run, start) DO UPDATE SET line = excluded.line",
        params![slug, start, metrics_line],
    )?;
    Ok(metrics_line)
}

/// Every metrics line recorded, oldest run first.
pub fn metrics_log(r: &Reader<'_>) -> rusqlite::Result<Vec<String>> {
    let conn = r.conn();
    let mut st = conn.prepare("SELECT line FROM metrics ORDER BY start, run")?;
    st.query_map([], |r| r.get(0))?.collect()
}

fn compress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(data)?;
    z.finish()
}

fn decompress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(data).read_to_end(&mut out)?;
    Ok(out)
}

/// Every file below `dir` as `(path relative to dir, contents)`. Symbolic
/// links are skipped, never followed: a link to an ancestor would loop and
/// one to a file outside the run would archive it.
fn files_below(dir: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&path)?));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The worktree's checks: nothing uncommitted, nothing unpushed. Returns its
/// branch.
fn worktree_clean(wt: &Path) -> Result<String, CleanupRefusal> {
    let branch = git::out(wt, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok_or(CleanupRefusal::DetachedHead)?;
    if !git::out(wt, &["status", "--porcelain"]).is_some_and(|s| s.is_empty()) {
        return Err(CleanupRefusal::UncommittedChanges);
    }
    if git::commit(wt, "@{upstream}").is_none() {
        return Err(CleanupRefusal::NoUpstream);
    }
    if git::out(wt, &["rev-list", "--count", "@{upstream}..HEAD"]).as_deref() != Some("0") {
        return Err(CleanupRefusal::UnpushedCommits);
    }
    Ok(branch)
}

/// What a clean-up did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cleaned {
    /// The archive written, `None` when there was nothing to archive.
    pub archive: Option<i64>,
    /// An earlier clean-up had archived the run and only its git and file
    /// steps were left, which this one finished.
    pub resumed: bool,
}

/// An archive whose worktree, branch or run directory is still to remove.
struct Pending {
    id: i64,
    worktree: Option<String>,
    branch: Option<String>,
    run_dir: bool,
}

fn pending(conn: &Connection, slug: &str) -> rusqlite::Result<Option<Pending>> {
    conn.query_row(
        "SELECT id, pending_worktree, pending_branch, pending_run_dir FROM archives
         WHERE run = ?1 AND (pending_worktree IS NOT NULL OR pending_branch IS NOT NULL
                             OR pending_run_dir <> 0)
         ORDER BY id DESC LIMIT 1",
        [slug],
        |r| {
            Ok(Pending {
                id: r.get(0)?,
                worktree: r.get(1)?,
                branch: r.get(2)?,
                run_dir: r.get::<_, i64>(3)? != 0,
            })
        },
    )
    .optional()
}

/// Removes the worktree, deletes the branch and removes the run directory,
/// each only when it is still there, and clears the archive's pending mark
/// of every step done, so that a rerun finishes the rest.
fn finish(conn: &Connection, layout: &Layout, slug: &str, todo: &Pending) -> Result<(), Error> {
    if let Some(wt) = &todo.worktree {
        let wt = Path::new(wt);
        if wt.is_dir() {
            let remove = [OsStr::new("worktree"), OsStr::new("remove"), wt.as_os_str()];
            git::run(&layout.root, &remove)
                .map_err(|e| Error::Refused(CleanupRefusal::WorktreeRemoveFailed(e)))?;
        }
        conn.execute(
            "UPDATE archives SET pending_worktree = NULL WHERE id = ?1",
            [todo.id],
        )?;
    }
    if let Some(branch) = &todo.branch {
        let _ = git::ok(&layout.root, &["worktree", "prune"]);
        let exists = git::ok(
            &layout.root,
            &[
                "rev-parse",
                "-q",
                "--verify",
                &format!("refs/heads/{branch}"),
            ],
        );
        if exists {
            git::run(&layout.root, &["branch", "-D", branch])
                .map_err(|e| Error::Refused(CleanupRefusal::BranchDeleteFailed(e)))?;
        }
        conn.execute(
            "UPDATE archives SET pending_branch = NULL WHERE id = ?1",
            [todo.id],
        )?;
    }
    if todo.run_dir {
        let run_dir = layout.run_dir(slug);
        if run_dir.is_dir() {
            std::fs::remove_dir_all(&run_dir)?;
        }
        conn.execute(
            "UPDATE archives SET pending_run_dir = 0 WHERE id = ?1",
            [todo.id],
        )?;
    }
    Ok(())
}

/// Cleans up a finished run: refuses while the worktree holds work its PR
/// would not carry or the run's CI is not green; else, in one transaction,
/// records the metrics and archives and deletes the cards and artifacts,
/// marking the worktree, the local branch and the run directory as still to
/// remove, and then removes them. A rerun after a failure there finishes
/// only what the archive still marks. Returns what was done.
pub fn cleanup(db: &mut Board, layout: &Layout, slug: &str, now: &str) -> Result<Cleaned, Error> {
    let conn = db.conn_mut();
    if let Some(todo) = pending(conn, slug)? {
        finish(conn, layout, slug, &todo)?;
        return Ok(Cleaned {
            archive: Some(todo.id),
            resumed: true,
        });
    }
    let wt = super::worktree(conn, layout, slug, slug)?;
    let branch = if wt.is_dir() {
        Some(worktree_clean(&wt).map_err(Error::Refused)?)
    } else {
        None
    };
    let run_dir = layout.run_dir(slug);
    let artifacts = if run_dir.is_dir() {
        files_below(&run_dir)?
    } else {
        Vec::new()
    };
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let has_parent = board::state(&tx, slug)?.is_some();
    if has_parent {
        metrics(&Writer::new(&tx), slug, now)?;
    }
    let mut cards: Vec<String> = Vec::new();
    if has_parent {
        cards.push(slug.to_owned());
    }
    cards.extend(board::stages(&tx, slug)?.into_iter().map(|(_, id, _)| id));
    let todo = Pending {
        id: 0,
        worktree: branch.as_ref().map(|_| wt.to_string_lossy().into_owned()),
        branch,
        run_dir: run_dir.is_dir(),
    };
    if cards.is_empty() && !run_dir.is_dir() {
        tx.commit()?;
        finish(conn, layout, slug, &todo)?;
        return Ok(Cleaned {
            archive: None,
            resumed: false,
        });
    }
    tx.execute(
        "INSERT INTO archives (run, archived_at, pending_worktree, pending_branch, pending_run_dir)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![slug, now, todo.worktree, todo.branch, todo.run_dir],
    )?;
    let id = tx.last_insert_rowid();
    let put = |path: String, data: &[u8]| -> Result<(), Error> {
        tx.execute(
            "INSERT INTO archive_files (archive, path, data) VALUES (?1, ?2, ?3)",
            params![id, path, compress(data)?],
        )?;
        Ok(())
    };
    for card in &cards {
        let text = board::show(&tx, card)?.unwrap_or_default();
        put(format!("cards/{card}.md"), text.as_bytes())?;
    }
    for (rel, data) in &artifacts {
        put(format!("artifacts/{rel}"), data)?;
    }
    for card in &cards {
        tx.execute("DELETE FROM cards WHERE id = ?1", [card])?;
    }
    tx.commit()?;
    finish(conn, layout, slug, &Pending { id, ..todo })?;
    Ok(Cleaned {
        archive: Some(id),
        resumed: false,
    })
}

/// A run's archives as `(id, archived_at)`, oldest first.
pub fn archives(r: &Reader<'_>, slug: &str) -> rusqlite::Result<Vec<(i64, String)>> {
    let conn = r.conn();
    let mut st = conn.prepare("SELECT id, archived_at FROM archives WHERE run = ?1 ORDER BY id")?;
    st.query_map([slug], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Writes archive `id` of run `slug` (its latest when `None`) below `dest`.
/// Returns the archive written, `None` when the run has none.
pub fn export(
    r: &Reader<'_>,
    slug: &str,
    id: Option<i64>,
    dest: &Path,
) -> Result<Option<(i64, PathBuf)>, Error> {
    let conn = r.conn();
    let id = match id {
        Some(id) => conn
            .query_row(
                "SELECT id FROM archives WHERE id = ?1 AND run = ?2",
                params![id, slug],
                |r| r.get(0),
            )
            .optional()?,
        None => conn.query_row("SELECT max(id) FROM archives WHERE run = ?1", [slug], |r| {
            r.get::<_, Option<i64>>(0)
        })?,
    };
    let Some(id) = id else {
        return Ok(None);
    };
    let mut st =
        conn.prepare("SELECT path, data FROM archive_files WHERE archive = ?1 ORDER BY path")?;
    let files: Vec<(String, Vec<u8>)> = st
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (rel, data) in files {
        if rel.split('/').any(|c| c == ".." || c.is_empty()) {
            continue;
        }
        let path = dest.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, decompress(&data)?)?;
    }
    Ok(Some((id, dest.to_path_buf())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(lines: &[&str]) -> Vec<Event> {
        lines.iter().map(|b| Event::parsed(b)).collect()
    }

    #[test]
    fn ut_ci_last_value_and_newer_waiver() {
        assert_eq!(ci(&ev(&["ci=0"])), Ok("0".into()));
        assert_eq!(ci(&ev(&["ci=0", "ci=1"])), Err(CiRefusal::CiRed));
        assert_eq!(ci(&ev(&["ci=1", "ci-waived x"])), Ok("waived".into()));
        assert_eq!(ci(&ev(&["ci-waived x", "ci=1"])), Err(CiRefusal::CiRed));
        assert_eq!(ci(&ev(&["ci-waived x"])), Err(CiRefusal::NoCi));
    }

    #[test]
    fn ut_metrics_count_stops_by_key_not_position() {
        let mut s = crate::store::Store::memory().unwrap();
        board::create(&mut s.conn, "r", board::State::Open, &[], "t").unwrap();
        let at = "2026-10-04T11:50";
        board::insert_body(&s.conn, "r", at, "stop gate=gate4 kind=approval");
        board::insert_body(&s.conn, "r", at, "stop kind=question gate=gate2");
        board::insert_body(&s.conn, "r", at, "note stop kind=blocked");
        board::log(&mut s.conn, "r", "ci=0", &[], at).unwrap();
        let line = metrics(&Writer::new(&s.conn), "r", "2026-10-04T12:00").unwrap();
        assert!(
            line.contains(" stops=2 question=1 approval=1 stage=0 blocked=0 "),
            "{line}"
        );
    }

    #[test]
    fn ut_stamps() {
        assert!(board::valid_stamp("2026-10-04T11:50"));
        assert!(!board::valid_stamp("2026-10-04"));
    }

    #[test]
    fn ut_compression_round_trips() {
        let data = b"---\nid: x\n---\n".repeat(50);
        let z = compress(&data).unwrap();
        assert!(z.len() < data.len());
        assert_eq!(decompress(&z).unwrap(), data);
    }
}
