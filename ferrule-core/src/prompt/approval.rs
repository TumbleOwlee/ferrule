//! The approval marker: which of a run's files the user was last shown,
//! so the approval that follows records exactly those revisions.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, TransactionBehavior, params};

use crate::artifacts;
use crate::config::Thresholds;
use crate::project::Layout;
use crate::store::Board;

/// The run whose artifact `path` is: a direct child of
/// `.ferrule/runs/<slug>/`.
#[must_use]
pub fn run_of(layout: &Layout, path: &Path) -> Option<String> {
    let path = path.canonicalize().ok()?;
    let dir = path.parent()?;
    let runs = layout.run_dir("x").parent()?.canonicalize().ok()?;
    (dir.parent()? == runs).then(|| dir.file_name()?.to_str().map(str::to_owned))?
}

/// Whether a user-facing summary exceeds its cap: `(lines, bytes)` when it
/// does. Only `*.summary.md` and `*.verdict.md` are capped.
#[must_use]
pub fn over_cap(path: &Path, text: &str, t: &Thresholds) -> Option<(usize, usize)> {
    let name = path.file_name()?.to_str()?;
    if !(name.ends_with(".summary.md") || name.ends_with(".verdict.md")) {
        return None;
    }
    // Lines as an editor shows them: a last line without a newline counts.
    let lines = text.lines().count();
    let bytes = text.len();
    (lines > t.summary_max_lines || bytes > t.summary_max_bytes).then_some((lines, bytes))
}

/// Replaces the marker of every run among `files` with that run's files
/// from this call, in order. Returns each file's run.
pub fn record(
    db: &mut Board,
    layout: &Layout,
    files: &[PathBuf],
    now: &str,
) -> rusqlite::Result<Vec<Option<String>>> {
    let conn = db.conn_mut();
    let runs: Vec<Option<String>> = files.iter().map(|f| run_of(layout, f)).collect();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut cleared: Vec<&str> = Vec::new();
    for (file, run) in files.iter().zip(&runs) {
        let Some(run) = run else { continue };
        if !cleared.contains(&run.as_str()) {
            tx.execute("DELETE FROM approvals WHERE run = ?1", [run])?;
            cleared.push(run);
        }
        let rev = std::fs::read_to_string(file)
            .ok()
            .and_then(|t| artifacts::rev(&t));
        let abs = file.canonicalize().unwrap_or_else(|_| file.clone());
        tx.execute(
            "INSERT INTO approvals (run, pos, path, rev, shown_at)
             VALUES (?1, (SELECT coalesce(max(pos), -1) + 1 FROM approvals WHERE run = ?1), ?2, ?3, ?4)",
            params![run, abs.to_string_lossy(), rev, now],
        )?;
    }
    tx.commit()?;
    Ok(runs)
}

/// The files the run's last stop showed, in order.
pub fn shown(conn: &Connection, run: &str) -> rusqlite::Result<Vec<PathBuf>> {
    let mut st = conn.prepare("SELECT path FROM approvals WHERE run = ?1 ORDER BY pos")?;
    st.query_map([run], |r| r.get::<_, String>(0).map(PathBuf::from))?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_caps_only_summaries() {
        let t = Thresholds::default();
        let long = "- line\n".repeat(30);
        assert_eq!(
            over_cap(Path::new("plan.summary.md"), &long, &t),
            Some((30, 210))
        );
        assert_eq!(over_cap(Path::new("plan.md"), &long, &t), None);
        assert_eq!(over_cap(Path::new("review.verdict.md"), "ok\n", &t), None);
        // One line over the cap, the last without a newline.
        let over = "- line\n".repeat(t.summary_max_lines) + "- last";
        assert!(over_cap(Path::new("a.summary.md"), &over, &t).is_some());
    }
}
