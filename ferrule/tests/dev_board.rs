//! A dev build works on its own copy of an older shared board.

mod golden;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use golden::Run;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A main checkout with a worktree `w` on `feat/w`.
struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
    wt: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("s");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join(".gitignore"), ".ferrule/\n").unwrap();
        git(&root, &["add", ".gitignore"]);
        git(&root, &["commit", "-q", "-m", "init"]);
        let wt = root.join(".ferrule/worktrees/w");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                wt.to_str().unwrap(),
                "-b",
                "feat/w",
                "main",
            ],
        );
        Repo {
            _dir: dir,
            root,
            wt,
        }
    }

    fn shared(&self) -> PathBuf {
        self.root.join(".ferrule/state.db")
    }

    /// An empty schema-0 shared board: a header, `user_version` 0.
    fn older(&self) {
        std::fs::create_dir_all(self.root.join(".ferrule")).unwrap();
        let c = rusqlite::Connection::open(self.shared()).unwrap();
        c.execute_batch("CREATE TABLE scratch (x); DROP TABLE scratch;")
            .unwrap();
    }
}

fn version(path: &Path) -> u32 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn tables(path: &Path) -> u32 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
        .unwrap()
}

fn ferrule(cwd: &Path, args: &[&str]) -> Output {
    Run::new(args).cwd(cwd).output()
}

fn notices(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("ferrule=dev-copy"))
        .map(str::to_owned)
        .collect()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// BO-R-003, BO-R-007, BO-E-001 — a dev build in a worktree takes its own
/// copy of an older shared board, says so once and leaves the shared board
/// untouched.
#[test]
fn it_dev_build_works_on_its_own_copy_in_a_worktree() {
    let r = Repo::new();
    r.older();
    let out = ferrule(&r.wt, &["board", "new", "demo"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let copy = r.wt.join(".ferrule/state.dev.db");
    let want = format!(
        "ferrule=dev-copy copy={} shared=0 schema={}",
        copy.display(),
        version(&copy)
    );
    assert_eq!(notices(&out), vec![want]);
    assert_eq!(version(&r.shared()), 0);
    assert_eq!(tables(&r.shared()), 0);
}

/// BO-R-005 — the copy is kept: later commands use it without a notice.
#[test]
fn it_dev_copy_persists_across_commands() {
    let r = Repo::new();
    r.older();
    assert!(ferrule(&r.wt, &["board", "new", "demo"]).status.success());
    let out = ferrule(&r.wt, &["board", "list"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("demo"), "{}", stdout(&out));
    assert!(notices(&out).is_empty());
    assert_eq!(version(&r.shared()), 0);
    assert_eq!(tables(&r.shared()), 0);
}

/// BO-R-005 — in the main checkout the copy is taken there.
#[test]
fn it_dev_build_in_the_main_checkout_takes_its_copy_there() {
    let r = Repo::new();
    r.older();
    let out = ferrule(&r.root, &["board", "new", "demo"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(r.root.join(".ferrule/state.dev.db").exists());
    assert_eq!(notices(&out).len(), 1);
    assert!(!r.wt.join(".ferrule/state.dev.db").exists());
}

/// BO-R-006 — the copy goes with its worktree.
#[test]
fn it_dev_copy_goes_with_its_worktree() {
    let r = Repo::new();
    r.older();
    assert!(ferrule(&r.wt, &["board", "new", "demo"]).status.success());
    let copy = r.wt.join(".ferrule/state.dev.db");
    assert!(copy.exists());
    git(&r.root, &["worktree", "remove", r.wt.to_str().unwrap()]);
    assert!(!copy.exists());
}

/// BO-R-004, BO-E-003 — with no board a dev build creates the shared board
/// and no copy.
#[test]
fn it_dev_build_shares_a_board_at_its_schema() {
    let r = Repo::new();
    let out = ferrule(&r.root, &["board", "new", "demo"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(r.shared().exists());
    assert!(!r.root.join(".ferrule/state.dev.db").exists());
    assert!(notices(&out).is_empty());
    let out = ferrule(&r.wt, &["board", "list"]);
    assert!(stdout(&out).contains("demo"), "{}", stdout(&out));
    assert!(notices(&out).is_empty());
}
