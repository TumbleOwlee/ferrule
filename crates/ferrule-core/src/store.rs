//! `.ferrule/state.db`: the board and everything else ferrule keeps about
//! runs, in `SQLite`. One connection per command, WAL so a reader never
//! waits on a writer, and a busy timeout for two writers at once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

pub use rusqlite::Error;

pub const DIR: &str = ".ferrule";
pub const FILE: &str = "state.db";

/// Schema migrations, append-only; the database's `user_version` counts
/// those applied.
const MIGRATIONS: &[&str] = &[
    r"
CREATE TABLE cards (
    id         TEXT PRIMARY KEY,
    run        TEXT NOT NULL,
    stage      TEXT,
    state      TEXT NOT NULL CHECK (state IN ('open', 'inprogress', 'inreview', 'done')),
    created_at TEXT NOT NULL
);
CREATE INDEX cards_run ON cards (run);
CREATE TABLE card_fields (
    card  TEXT NOT NULL REFERENCES cards (id) ON DELETE CASCADE,
    pos   INTEGER NOT NULL,
    key   TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (card, key)
);
CREATE TABLE events (
    id   INTEGER PRIMARY KEY,
    card TEXT NOT NULL REFERENCES cards (id) ON DELETE CASCADE,
    at   TEXT NOT NULL,
    name TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE INDEX events_card ON events (card, id);
",
    r"
CREATE TABLE archives (
    id          INTEGER PRIMARY KEY,
    run         TEXT NOT NULL,
    archived_at TEXT NOT NULL
);
CREATE INDEX archives_run ON archives (run, id);
CREATE TABLE archive_files (
    archive INTEGER NOT NULL REFERENCES archives (id) ON DELETE CASCADE,
    path    TEXT NOT NULL,
    data    BLOB NOT NULL,
    PRIMARY KEY (archive, path)
);
CREATE TABLE metrics (
    run   TEXT NOT NULL,
    start TEXT NOT NULL,
    line  TEXT NOT NULL,
    PRIMARY KEY (run, start)
);
",
    r"
CREATE TABLE gauntlet_cache (
    run  TEXT PRIMARY KEY,
    tree TEXT NOT NULL,
    cov  TEXT NOT NULL,
    at   TEXT NOT NULL
);
",
    r"
CREATE TABLE approvals (
    run      TEXT NOT NULL,
    pos      INTEGER NOT NULL,
    path     TEXT NOT NULL,
    rev      INTEGER,
    shown_at TEXT NOT NULL,
    PRIMARY KEY (run, pos)
);
",
];

#[derive(Debug)]
pub enum OpenError {
    /// No `.ferrule/state.db` and none asked for.
    Missing(PathBuf),
    /// The database was written by a newer ferrule.
    Newer {
        found: u32,
        known: u32,
    },
    Sql(Error),
    Io(std::io::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Missing(p) => write!(f, "no state database at {}", p.display()),
            OpenError::Newer { found, known } => write!(
                f,
                "state database schema {found} is newer than this ferrule's {known}; upgrade ferrule"
            ),
            OpenError::Sql(e) => write!(f, "state database: {e}"),
            OpenError::Io(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<Error> for OpenError {
    fn from(e: Error) -> Self {
        OpenError::Sql(e)
    }
}

pub struct Store {
    pub conn: Connection,
}

impl Store {
    /// The database under `root/.ferrule/`, migrated to the current schema;
    /// created first when `create` is set.
    pub fn open(root: &Path, create: bool) -> Result<Self, OpenError> {
        let dir = root.join(DIR);
        let path = dir.join(FILE);
        if !path.exists() {
            if !create {
                return Err(OpenError::Missing(path));
            }
            std::fs::create_dir_all(&dir).map_err(OpenError::Io)?;
        }
        let mut conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        Ok(Store { conn })
    }

    /// An in-memory database for tests.
    pub fn memory() -> Result<Self, OpenError> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        Ok(Store { conn })
    }
}

fn migrate(conn: &mut Connection) -> Result<(), OpenError> {
    let known = u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX);
    let found: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found > known {
        return Err(OpenError::Newer { found, known });
    }
    if found == known {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let found: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i + 1)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_open_creates_and_migrates_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            Store::open(dir.path(), false),
            Err(OpenError::Missing(_))
        ));
        let s = Store::open(dir.path(), true).unwrap();
        let v: u32 = s
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        drop(s);
        Store::open(dir.path(), false).unwrap();
    }

    #[test]
    fn ut_newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path(), true).unwrap();
        s.conn.pragma_update(None, "user_version", 99).unwrap();
        drop(s);
        assert!(matches!(
            Store::open(dir.path(), false),
            Err(OpenError::Newer { found: 99, .. })
        ));
    }
}
