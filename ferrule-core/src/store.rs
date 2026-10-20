//! `.ferrule/state.db`: the board and everything else ferrule keeps about
//! runs, in `SQLite`. One connection per command, WAL so a reader never
//! waits on a writer, and a busy timeout for two writers at once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

pub use rusqlite::Error;

pub const DIR: &str = ".ferrule";
pub const FILE: &str = "state.db";

/// One schema migration: SQL, or Rust for what SQL alone cannot do. Each
/// runs inside the migrating transaction.
enum Step {
    Sql(&'static str),
    Rust(fn(&Connection) -> Result<(), Error>),
}

/// Schema migrations, append-only; the database's `user_version` counts
/// those applied.
const MIGRATIONS: &[Step] = &[
    Step::Sql(
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
    ),
    Step::Sql(
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
    ),
    Step::Sql(
        r"
CREATE TABLE gauntlet_cache (
    run  TEXT PRIMARY KEY,
    tree TEXT NOT NULL,
    cov  TEXT NOT NULL,
    at   TEXT NOT NULL
);
",
    ),
    Step::Sql(
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
    ),
    Step::Sql(
        r"
ALTER TABLE events ADD COLUMN value TEXT;
ALTER TABLE events ADD COLUMN text TEXT;
CREATE TABLE event_args (
    event INTEGER NOT NULL REFERENCES events (id) ON DELETE CASCADE,
    idx   INTEGER NOT NULL,
    key   TEXT,
    value TEXT NOT NULL,
    PRIMARY KEY (event, idx)
);
CREATE INDEX events_card_name ON events (card, name, id);
",
    ),
    Step::Rust(crate::card::board::backfill_args),
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
    migrate_to(conn, MIGRATIONS.len())
}

/// Applies the first `upto` migrations not yet applied.
fn migrate_to(conn: &mut Connection, upto: usize) -> Result<(), OpenError> {
    let known = u32::try_from(upto).unwrap_or(u32::MAX);
    let found: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found > known {
        return Err(OpenError::Newer { found, known });
    }
    if found == known {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let found: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, step) in MIGRATIONS[..upto].iter().enumerate().skip(found as usize) {
        match step {
            Step::Sql(sql) => tx.execute_batch(sql)?,
            Step::Rust(f) => f(&tx)?,
        }
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
    fn ut_event_args_are_backfilled_from_bodies() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate_to(&mut conn, 4).unwrap();
        conn.execute(
            "INSERT INTO cards (id, run, state, created_at) VALUES ('r', 'r', 'open', 't')",
            [],
        )
        .unwrap();
        let bodies = [
            ("note", "note gauntlet=pass sha=x"),
            ("files+=", "files+=src/a.rs reason=\"a b\""),
            ("spec-delta", "spec-delta MB-R-1 added origin=impl"),
            ("stop", "stop gate=gate2 kind=approval"),
            ("gone", "gone with=words"),
        ];
        for (name, body) in bodies {
            conn.execute(
                "INSERT INTO events (card, at, name, body) VALUES ('r', 't', ?1, ?2)",
                [name, body],
            )
            .unwrap();
        }
        migrate(&mut conn).unwrap();
        let row = |id: i64| -> (Option<String>, Option<String>, String) {
            conn.query_row(
                "SELECT value, text, body FROM events WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        let args = |id| crate::card::board::event_args(&conn, id).unwrap();
        let kv = |k: Option<&str>, v: &str| (k.map(str::to_owned), v.to_owned());

        assert_eq!(
            row(1),
            (None, Some("gauntlet=pass sha=x".into()), bodies[0].1.into())
        );
        assert!(args(1).is_empty());
        assert_eq!(row(2).0.as_deref(), Some("src/a.rs"));
        assert_eq!(args(2), [kv(Some("reason"), "a b")]);
        assert_eq!(
            args(3),
            [
                kv(None, "MB-R-1"),
                kv(None, "added"),
                kv(Some("origin"), "impl")
            ]
        );
        assert_eq!(row(3).1, None);
        assert_eq!(
            args(4),
            [kv(Some("gate"), "gate2"), kv(Some("kind"), "approval")]
        );
        assert_eq!(row(5).1.as_deref(), Some("with=words"));
        assert!(args(5).is_empty());
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
