//! `.ferrule/state.db`: the board and everything else ferrule keeps about
//! runs, in `SQLite`. One connection per command, WAL so a reader never
//! waits on a writer, and a busy timeout for two writers at once. A dev
//! build may work on `.ferrule/state.dev.db` instead.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

pub use rusqlite::Error;

pub use crate::project::DIR;
pub const FILE: &str = "state.db";
pub const DEV_FILE: &str = "state.dev.db";

/// Which kind of binary is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Release,
    Dev,
}

impl Build {
    /// `Dev` when debug assertions are on.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(debug_assertions) {
            Build::Dev
        } else {
            Build::Release
        }
    }
}

/// Where a command runs and with what kind of build: enough to resolve
/// the board it works on.
#[derive(Debug, Clone)]
pub struct Site {
    /// The main checkout: the shared board is `shared/.ferrule/state.db`.
    pub shared: PathBuf,
    /// The checkout the command runs in: a dev build's copy is
    /// `checkout/.ferrule/state.dev.db`. Equal to `shared` in the main checkout.
    pub checkout: PathBuf,
    pub build: Build,
}

impl Site {
    #[must_use]
    pub fn new(shared: PathBuf, checkout: PathBuf, build: Build) -> Self {
        Site {
            shared,
            checkout,
            build,
        }
    }

    /// The main checkout, the current checkout and this binary's build.
    #[must_use]
    pub fn current() -> Self {
        Site::new(
            crate::project::main_checkout(),
            crate::project::toplevel(),
            Build::current(),
        )
    }

    fn shared_file(&self) -> PathBuf {
        self.shared.join(DIR).join(FILE)
    }

    fn copy_file(&self) -> PathBuf {
        self.checkout.join(DIR).join(DEV_FILE)
    }
}

/// Which database a [`Board`] was opened on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The shared board (always for a release build and an in-memory board).
    Shared,
    /// A dev build's copy taken by an earlier command.
    Copy { path: PathBuf },
    /// A dev build's copy this open took from an older shared board.
    Taken {
        path: PathBuf,
        shared: u32,
        schema: u32,
    },
}

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
    Step::Sql("ALTER TABLE archives ADD COLUMN abandoned INTEGER NOT NULL DEFAULT 0;"),
];

#[derive(Debug)]
pub enum OpenError {
    /// No database at the path and none asked for.
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
    /// The database file at `path`, migrated to the current schema; created
    /// first (with its directory) when `create` is set.
    pub fn open(path: &Path, create: bool) -> Result<Self, OpenError> {
        if !path.exists() {
            if !create {
                return Err(OpenError::Missing(path.to_path_buf()));
            }
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(OpenError::Io)?;
            }
        }
        let mut conn = Connection::open(path)?;
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
    origin: Origin,
}

impl Board {
    /// The board `site` resolves to; `None` when there is none, which
    /// reads as an empty board. Never creates one. A release build uses the
    /// shared board. A dev build uses its copy when it has one, else the
    /// shared board when its schema equals the build's, else takes a copy
    /// of an older one; a newer one is refused.
    pub fn open_existing(site: &Site) -> Result<Option<Self>, OpenError> {
        resolve(site, false)
    }

    /// The board `site` resolves to, created at the shared path when there
    /// is none: for the commands that start a board. Resolves as
    /// [`Board::open_existing`] otherwise.
    pub fn create(site: &Site) -> Result<Self, OpenError> {
        resolve(site, true)?
            .ok_or_else(|| OpenError::Io(std::io::Error::other("no board resolved for create")))
    }

    /// An empty in-memory board.
    pub fn memory() -> Result<Self, OpenError> {
        Ok(Board {
            store: Store::memory()?,
            origin: Origin::Shared,
        })
    }

    /// Which database this board is on.
    #[must_use]
    pub fn origin(&self) -> &Origin {
        &self.origin
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

fn known_schema() -> u32 {
    u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX)
}

fn resolve(site: &Site, create: bool) -> Result<Option<Board>, OpenError> {
    let shared = site.shared_file();
    let open = |path: &Path, create: bool, origin: Origin| match Store::open(path, create) {
        Ok(store) => Ok(Some(Board { store, origin })),
        Err(OpenError::Missing(_)) => Ok(None),
        Err(e) => Err(e),
    };
    if site.build == Build::Release {
        return open(&shared, create, Origin::Shared);
    }
    let copy = site.copy_file();
    if copy.exists() {
        return open(&copy, false, Origin::Copy { path: copy.clone() });
    }
    let known = known_schema();
    let Some(found) = schema(&shared)? else {
        return if create {
            open(&shared, true, Origin::Shared)
        } else {
            Ok(None)
        };
    };
    match found.cmp(&known) {
        std::cmp::Ordering::Equal => open(&shared, false, Origin::Shared),
        std::cmp::Ordering::Greater => Err(OpenError::Newer { found, known }),
        std::cmp::Ordering::Less => {
            take_copy(&shared, &copy)?;
            open(
                &copy,
                false,
                Origin::Taken {
                    path: copy.clone(),
                    shared: found,
                    schema: known,
                },
            )
        }
    }
}

/// A database opened without creating it.
fn open_plain(path: &Path) -> Result<Connection, OpenError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

/// The schema version of the database at `path`; `None` when absent.
fn schema(path: &Path) -> Result<Option<u32>, OpenError> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = open_plain(path)?;
    let found = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    Ok(Some(found))
}

/// Snapshots `shared` into `copy`: written beside it, then renamed, so a
/// reader never sees a half-written copy. Two takers at once leave equal
/// snapshots, the last rename winning. A failed take removes its temporary
/// file.
fn take_copy(shared: &Path, copy: &Path) -> Result<(), OpenError> {
    if let Some(dir) = copy.parent() {
        std::fs::create_dir_all(dir).map_err(OpenError::Io)?;
    }
    let tmp = copy.with_file_name(format!("{DEV_FILE}.{}.tmp", std::process::id()));
    if tmp.exists() {
        std::fs::remove_file(&tmp).map_err(OpenError::Io)?;
    }
    let tmp_str = tmp
        .to_str()
        .ok_or_else(|| OpenError::Io(std::io::Error::other("non-UTF-8 path for the board copy")))?;
    let snapshot = || -> Result<(), OpenError> {
        open_plain(shared)?.execute("VACUUM INTO ?1", [tmp_str])?;
        std::fs::rename(&tmp, copy).map_err(OpenError::Io)?;
        Ok(())
    };
    let taken = snapshot();
    if taken.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    taken
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

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| Path::new(n).extension().is_some_and(|x| x == "tmp"))
            .collect()
    }

    /// BO-E-005 — a rename that fails leaves no temporary copy behind.
    #[test]
    fn ut_failed_rename_leaves_no_temporary_copy() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        let wt = dir.path().join("wt");
        let shared = main.join(DIR).join(FILE);
        drop(Store::open(&shared, true).unwrap());
        let copy = wt.join(DIR).join(DEV_FILE);
        std::fs::create_dir_all(&copy).unwrap();
        std::fs::write(copy.join("keep"), b"x").unwrap();
        assert!(matches!(take_copy(&shared, &copy), Err(OpenError::Io(_))));
        assert!(leftovers(&wt.join(DIR)).is_empty());
    }

    /// BO-E-005 — a snapshot that fails leaves no temporary copy and no copy.
    #[test]
    fn ut_failed_snapshot_leaves_no_temporary_copy() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("main").join(DIR).join(FILE);
        std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
        std::fs::write(&shared, b"not a database").unwrap();
        let wt = dir.path().join("wt");
        let copy = wt.join(DIR).join(DEV_FILE);
        assert!(take_copy(&shared, &copy).is_err());
        assert!(leftovers(&wt.join(DIR)).is_empty());
        assert!(!copy.exists());
    }

    #[test]
    fn ut_open_creates_and_migrates_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(DIR).join(FILE);
        assert!(matches!(
            Store::open(&file, false),
            Err(OpenError::Missing(_))
        ));
        let s = Store::open(&file, true).unwrap();
        let v: u32 = s
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        drop(s);
        Store::open(&file, false).unwrap();
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
        let file = dir.path().join(DIR).join(FILE);
        let s = Store::open(&file, true).unwrap();
        s.conn.pragma_update(None, "user_version", 99).unwrap();
        drop(s);
        assert!(matches!(
            Store::open(&file, false),
            Err(OpenError::Newer { found: 99, .. })
        ));
    }

    #[test]
    fn ut_board_open_without_a_database_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let site = Site::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            Build::Release,
        );
        assert!(Board::open_existing(&site).unwrap().is_none());
        assert!(Board::create(&site).is_ok());
        assert!(Board::open_existing(&site).unwrap().is_some());
    }

    #[test]
    fn ut_read_sees_one_snapshot_while_another_connection_commits() {
        use crate::card::board::State;

        let dir = tempfile::tempdir().unwrap();
        let site = Site::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            Build::Release,
        );
        let board = Board::create(&site).unwrap();
        let mut other = Board::open_existing(&site).unwrap().unwrap();
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

    fn site(shared: &Path, checkout: &Path, build: Build) -> Site {
        Site::new(shared.to_path_buf(), checkout.to_path_buf(), build)
    }

    /// A shared board at schema 1 holding card `r`.
    fn older(root: &Path) {
        std::fs::create_dir_all(root.join(DIR)).unwrap();
        let mut conn = Connection::open(root.join(DIR).join(FILE)).unwrap();
        migrate_to(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO cards (id, run, state, created_at) VALUES ('r', 'r', 'open', 't')",
            [],
        )
        .unwrap();
    }

    fn version(path: &Path) -> u32 {
        Connection::open(path)
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    fn has_card(path: &Path, id: &str) -> bool {
        let n: i64 = Connection::open(path)
            .unwrap()
            .query_row("SELECT count(*) FROM cards WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        n == 1
    }

    fn known() -> u32 {
        u32::try_from(MIGRATIONS.len()).unwrap()
    }

    /// BO-R-001, S802 — a release build migrates an older board in place.
    #[test]
    fn ut_release_migrates_an_older_board_in_place() {
        let main = tempfile::tempdir().unwrap();
        older(main.path());
        let s = site(main.path(), main.path(), Build::Release);
        let board = Board::open_existing(&s).unwrap().unwrap();
        assert_eq!(board.origin(), &Origin::Shared);
        assert_eq!(version(&s.shared_file()), known());
        assert_eq!(
            board.read(|r| r.state("r")).unwrap(),
            Some(crate::card::board::State::Open)
        );
    }

    /// BO-R-001 — a release build ignores a dev copy.
    #[test]
    fn ut_release_ignores_a_copy() {
        let main = tempfile::tempdir().unwrap();
        let s = site(main.path(), main.path(), Build::Release);
        Store::open(&s.shared_file(), true).unwrap();
        Store::open(&s.copy_file(), true).unwrap();
        let board = Board::open_existing(&s).unwrap().unwrap();
        assert_eq!(board.origin(), &Origin::Shared);
    }

    /// BO-R-002, BO-R-003 — a dev build copies an older board and migrates
    /// only the copy.
    #[test]
    fn ut_dev_takes_a_migrated_copy_of_an_older_board() {
        let (main, wt) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        older(main.path());
        let s = site(main.path(), wt.path(), Build::Dev);
        let board = Board::open_existing(&s).unwrap().unwrap();
        assert_eq!(
            board.origin(),
            &Origin::Taken {
                path: s.copy_file(),
                shared: 1,
                schema: known()
            }
        );
        assert_eq!(version(&s.shared_file()), 1);
        assert_eq!(version(&s.copy_file()), known());
        assert_eq!(
            board.read(|r| r.state("r")).unwrap(),
            Some(crate::card::board::State::Open)
        );
    }

    /// BO-R-004, S803 — a dev build shares a board at its own schema.
    #[test]
    fn ut_dev_shares_a_board_at_its_schema() {
        use crate::card::board::State;

        let (main, wt) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        Board::create(&site(main.path(), main.path(), Build::Release)).unwrap();
        let s = site(main.path(), wt.path(), Build::Dev);
        let mut dev = Board::open_existing(&s).unwrap().unwrap();
        assert_eq!(dev.origin(), &Origin::Shared);
        dev.write(|w| w.create("x", State::Open, &[], "t")).unwrap();
        let rel = Board::open_existing(&site(main.path(), main.path(), Build::Release))
            .unwrap()
            .unwrap();
        assert_eq!(rel.read(|r| r.state("x")).unwrap(), Some(State::Open));
        assert!(!s.copy_file().exists());
    }

    /// BO-R-005 — a taken copy is used by later opens in that checkout.
    #[test]
    fn ut_dev_reuses_its_copy() {
        let (main, wt) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        older(main.path());
        let s = site(main.path(), wt.path(), Build::Dev);
        Board::open_existing(&s).unwrap().unwrap();
        let again = Board::open_existing(&s).unwrap().unwrap();
        assert_eq!(
            again.origin(),
            &Origin::Copy {
                path: s.copy_file()
            }
        );
        assert_eq!(version(&s.shared_file()), 1);
    }

    /// BO-E-001 — a write on the copy never reaches the shared board.
    #[test]
    fn ut_copy_writes_never_reach_the_shared_board() {
        use crate::card::board::State;

        let (main, wt) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        older(main.path());
        let s = site(main.path(), wt.path(), Build::Dev);
        let mut board = Board::open_existing(&s).unwrap().unwrap();
        board
            .write(|w| w.create("x", State::Open, &[], "t"))
            .unwrap();
        assert!(has_card(&s.copy_file(), "x"));
        assert!(!has_card(&s.shared_file(), "x"));
    }

    /// BO-E-002 — a kept copy outlives the shared board reaching its schema.
    #[test]
    fn ut_kept_copy_wins_after_the_shared_board_catches_up() {
        let (main, wt) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        older(main.path());
        let s = site(main.path(), wt.path(), Build::Dev);
        Board::open_existing(&s).unwrap().unwrap();
        Board::open_existing(&site(main.path(), main.path(), Build::Release))
            .unwrap()
            .unwrap();
        assert_eq!(version(&s.shared_file()), known());
        let board = Board::open_existing(&s).unwrap().unwrap();
        assert_eq!(
            board.origin(),
            &Origin::Copy {
                path: s.copy_file()
            }
        );
    }

    /// BO-E-003 — a dev build with no board creates the shared one.
    #[test]
    fn ut_dev_creates_the_shared_board_when_there_is_none() {
        let (main, wt) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let s = site(main.path(), wt.path(), Build::Dev);
        assert!(Board::open_existing(&s).unwrap().is_none());
        let board = Board::create(&s).unwrap();
        assert_eq!(board.origin(), &Origin::Shared);
        assert_eq!(version(&s.shared_file()), known());
        assert!(!s.copy_file().exists());
    }

    /// BO-E-004 — either build refuses a board newer than its own.
    #[test]
    fn ut_newer_board_is_refused_by_either_build() {
        let (main, wt) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let s = site(main.path(), wt.path(), Build::Dev);
        Store::open(&s.shared_file(), true)
            .unwrap()
            .conn
            .pragma_update(None, "user_version", 99)
            .unwrap();
        for build in [Build::Release, Build::Dev] {
            let s = site(main.path(), wt.path(), build);
            assert!(matches!(
                Board::open_existing(&s),
                Err(OpenError::Newer { found: 99, .. })
            ));
        }
        assert!(!s.copy_file().exists());
    }

    /// BO-R-002 — a build with debug assertions on is a dev build.
    #[test]
    fn ut_build_is_dev_with_debug_assertions() {
        let expect = if cfg!(debug_assertions) {
            Build::Dev
        } else {
            Build::Release
        };
        assert_eq!(Build::current(), expect);
    }
}
