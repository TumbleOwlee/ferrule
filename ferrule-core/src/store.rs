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
    Step::Sql(
        r"
ALTER TABLE archives ADD COLUMN pending_worktree TEXT;
ALTER TABLE archives ADD COLUMN pending_branch TEXT;
ALTER TABLE archives ADD COLUMN pending_run_dir INTEGER NOT NULL DEFAULT 0;
",
    ),
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

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OpenError::Sql(e) => Some(e),
            OpenError::Io(e) => Some(e),
            OpenError::Missing(_) | OpenError::Newer { .. } => None,
        }
    }
}

impl From<Error> for OpenError {
    fn from(e: Error) -> Self {
        OpenError::Sql(e)
    }
}

/// The open database. Its connection stays inside this crate: callers go
/// through [`Board`], whose reads and writes are each one transaction.
pub(crate) struct Store {
    pub(crate) conn: Connection,
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

/// The board's database as callers outside this crate see it: no SQL and
/// no connection, only reads in one snapshot, writes in one transaction,
/// and the core operations that take the board whole.
pub struct Board {
    store: Store,
}

impl Board {
    /// The database under `root/.ferrule/`; `None` when there is none and
    /// `create` is not set, which reads as an empty board.
    pub fn open(root: &Path, create: bool) -> Result<Option<Self>, OpenError> {
        match Store::open(root, create) {
            Ok(store) => Ok(Some(Board { store })),
            Err(OpenError::Missing(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// An empty in-memory board.
    pub fn memory() -> Result<Self, OpenError> {
        Ok(Board {
            store: Store::memory()?,
        })
    }

    /// `f` inside one deferred read transaction: every read it makes sees
    /// the same snapshot, whatever another connection commits meanwhile.
    pub fn read<T, E: From<Error>>(
        &self,
        f: impl FnOnce(&Reader<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        snapshot(&self.store.conn, f)
    }

    /// `f` inside one immediate write transaction, committed when it
    /// returns `Ok` and rolled back otherwise.
    pub fn write<T, E: From<Error>>(
        &mut self,
        f: impl FnOnce(&Writer<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let tx = self
            .store
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let out = f(&Writer {
            reader: Reader { conn: &tx },
        })?;
        tx.commit()?;
        Ok(out)
    }

    /// The connection, for core operations that bring their own
    /// transactions or run commands between them.
    pub(crate) fn conn(&self) -> &Connection {
        &self.store.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.store.conn
    }
}

/// `f` over `conn` in one read snapshot: a deferred transaction of its
/// own, or the transaction `conn` is already in.
pub(crate) fn snapshot<T, E: From<Error>>(
    conn: &Connection,
    f: impl FnOnce(&Reader<'_>) -> Result<T, E>,
) -> Result<T, E> {
    if !conn.is_autocommit() {
        return f(&Reader { conn });
    }
    let tx = conn.unchecked_transaction()?;
    let out = f(&Reader { conn: &tx })?;
    tx.commit()?;
    Ok(out)
}

/// Reads inside [`Board::read`], or inside a [`Writer`]'s transaction.
pub struct Reader<'a> {
    conn: &'a Connection,
}

impl<'a> Reader<'a> {
    /// A reader over `conn`, for core code that already holds one.
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Reader { conn }
    }

    pub(crate) fn conn(&self) -> &'a Connection {
        self.conn
    }
}

/// Writes inside [`Board::write`]; it reads through [`Reader`]'s methods.
pub struct Writer<'a> {
    reader: Reader<'a>,
}

impl<'a> Writer<'a> {
    /// A writer over `conn`, for core code that already holds one inside
    /// its own transaction.
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Writer {
            reader: Reader { conn },
        }
    }
}

impl<'a> std::ops::Deref for Writer<'a> {
    type Target = Reader<'a>;

    fn deref(&self) -> &Reader<'a> {
        &self.reader
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

    #[test]
    fn ut_board_open_without_a_database_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Board::open(dir.path(), false).unwrap().is_none());
        assert!(Board::open(dir.path(), true).unwrap().is_some());
        assert!(Board::open(dir.path(), false).unwrap().is_some());
    }

    #[test]
    fn ut_read_sees_one_snapshot_while_another_connection_commits() {
        use crate::card::board::State;

        let dir = tempfile::tempdir().unwrap();
        let board = Board::open(dir.path(), true).unwrap().unwrap();
        let mut other = Board::open(dir.path(), false).unwrap().unwrap();
        let (before, during) = board
            .read(|r| {
                let before = r.state("r")?;
                other
                    .write(|w| w.create("r", State::Open, &[], "t"))
                    .unwrap();
                Ok::<_, Error>((before, r.state("r")?))
            })
            .unwrap();
        assert_eq!((before, during), (None, None));
        assert_eq!(board.read(|r| r.state("r")).unwrap(), Some(State::Open));
    }

    #[test]
    fn ut_write_rolls_back_on_error() {
        use crate::card::board::{CardError, State};

        let mut board = Board::memory().unwrap();
        let r = board.write(|w| {
            w.create("r", State::Open, &[], "t")?;
            w.create("r", State::Open, &[], "t")
        });
        assert!(matches!(r, Err(CardError::Exists)));
        assert_eq!(board.read(|r| r.state("r")).unwrap(), None);
    }
}
