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
}

#[derive(Debug)]
pub enum Error {
    Card,
    Ci(CiRefusal),
    /// A clean-up refused before it changed anything.
    Refused(&'static str),
    Io(std::io::Error),
    Sql(rusqlite::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Card => f.write_str("the run has no card"),
            Error::Ci(r) => write!(f, "CI refused: {}", r.as_str()),
            Error::Refused(r) => write!(f, "refused: {r}"),
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

/// `YYYY-MM-DDTHH:MM` at the start of a log line.
fn is_stamp(at: &str) -> bool {
    let b = at.as_bytes();
    b.len() == 16
        && b.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 => c == b'-',
            10 => c == b'T',
            13 => c == b':',
            _ => c.is_ascii_digit(),
        })
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
pub fn metrics(conn: &Connection, slug: &str, now: &str) -> Result<String, Error> {
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
        .filter(|at| is_stamp(at))
        .min()
        .unwrap_or("-")
        .to_owned();
    let first_red = stage_events
        .iter()
        .filter(|e| is_stamp(&e.at) && e.typed() == Typed::TestRed)
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
pub fn metrics_log(conn: &Connection) -> rusqlite::Result<Vec<String>> {
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

/// Every file below `dir` as `(path relative to dir, contents)`.
fn files_below(dir: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
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
fn worktree_clean(wt: &Path) -> Result<String, &'static str> {
    let branch =
        git::out(wt, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok_or("detached-head")?;
    if !git::out(wt, &["status", "--porcelain"]).is_some_and(|s| s.is_empty()) {
        return Err("uncommitted-changes");
    }
    if git::commit(wt, "@{upstream}").is_none() {
        return Err("no-upstream");
    }
    if git::out(wt, &["rev-list", "--count", "@{upstream}..HEAD"]).as_deref() != Some("0") {
        return Err("unpushed-commits");
    }
    Ok(branch)
}

/// Cleans up a finished run: refuses while the worktree holds work its PR
/// would not carry or the run's CI is not green; else records the metrics,
/// removes the worktree and the local branch, and archives the cards and
/// artifacts. Returns the archive id, `None` when there was nothing to
/// archive.
pub fn cleanup(
    conn: &mut Connection,
    layout: &Layout,
    slug: &str,
    now: &str,
) -> Result<Option<i64>, Error> {
    let wt = layout.worktree(slug);
    let branch = if wt.is_dir() {
        Some(worktree_clean(&wt).map_err(Error::Refused)?)
    } else {
        None
    };
    let has_parent = board::state(conn, slug)?.is_some();
    if has_parent {
        metrics(conn, slug, now)?;
    }
    if let Some(branch) = &branch {
        let remove = [OsStr::new("worktree"), OsStr::new("remove"), wt.as_os_str()];
        if !git::ok(&layout.root, &remove) {
            return Err(Error::Refused("worktree-remove-failed"));
        }
        if !git::ok(&layout.root, &["branch", "-D", branch]) {
            return Err(Error::Refused("branch-delete-failed"));
        }
    }
    let run_dir = layout.run_dir(slug);
    let mut cards: Vec<String> = Vec::new();
    if has_parent {
        cards.push(slug.to_owned());
    }
    cards.extend(board::stages(conn, slug)?.into_iter().map(|(_, id, _)| id));
    let artifacts = if run_dir.is_dir() {
        files_below(&run_dir)?
    } else {
        Vec::new()
    };
    if cards.is_empty() && !run_dir.is_dir() {
        return Ok(None);
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO archives (run, archived_at) VALUES (?1, ?2)",
        params![slug, now],
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
    if run_dir.is_dir() {
        std::fs::remove_dir_all(&run_dir)?;
    }
    Ok(Some(id))
}

/// A run's archives as `(id, archived_at)`, oldest first.
pub fn archives(conn: &Connection, slug: &str) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut st = conn.prepare("SELECT id, archived_at FROM archives WHERE run = ?1 ORDER BY id")?;
    st.query_map([slug], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Writes archive `id` of run `slug` (its latest when `None`) below `dest`.
/// Returns the archive written, `None` when the run has none.
pub fn export(
    conn: &Connection,
    slug: &str,
    id: Option<i64>,
    dest: &Path,
) -> Result<Option<(i64, PathBuf)>, Error> {
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
        let line = metrics(&s.conn, "r", "2026-10-04T12:00").unwrap();
        assert!(
            line.contains(" stops=2 question=1 approval=1 stage=0 blocked=0 "),
            "{line}"
        );
    }

    #[test]
    fn ut_stamps() {
        assert!(is_stamp("2026-10-04T11:50"));
        assert!(!is_stamp("2026-10-04"));
    }

    #[test]
    fn ut_compression_round_trips() {
        let data = b"---\nid: x\n---\n".repeat(50);
        let z = compress(&data).unwrap();
        assert!(z.len() < data.len());
        assert_eq!(decompress(&z).unwrap(), data);
    }
}
