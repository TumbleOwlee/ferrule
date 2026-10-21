//! `run rebuild`: a lost board recreated from a fake `gh`'s issue and
//! draft PR and a branch on a real origin.

mod golden;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use golden::{FakeGh, GH_FIELDS, Run, check, fixtures, gh_json};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A checkout whose origin holds main with a spec and branch `docs/rb`
/// with two commits changing it; the branch is not checked out locally.
struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
    sha0: String,
    sha1: String,
    gh: FakeGh,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin.git");
        let root = dir.path().join("r");
        git(
            dir.path(),
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                origin.to_str().unwrap(),
            ],
        );
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                root.to_str().unwrap(),
            ],
        );
        let specs = root.join("docs/specs/modbus");
        std::fs::create_dir_all(&specs).unwrap();
        std::fs::write(
            specs.join("requirements.md"),
            "## Client\n\n**MB-R-001** — The client connects.\n",
        )
        .unwrap();
        git(&root, &["checkout", "-q", "-b", "main"]);
        git(&root, &["add", "docs"]);
        git(&root, &["commit", "-qm", "spec"]);
        git(&root, &["push", "-q", "origin", "main"]);
        git(&root, &["checkout", "-q", "-b", "docs/rb"]);
        std::fs::write(
            specs.join("requirements.md"),
            "## Client\n\n**MB-R-001** — The client connects over TCP.\n",
        )
        .unwrap();
        git(&root, &["commit", "-qam", "change"]);
        let sha0 = git(&root, &["rev-parse", "--short", "HEAD"]);
        std::fs::write(
            specs.join("requirements.md"),
            "## Client\n\n**MB-R-001** — The client connects over TCP.\n**MB-R-002** — The client polls.\n",
        )
        .unwrap();
        std::fs::write(
            specs.join("edge-cases.md"),
            "## Client\n\n| ID | Condition | Behavior |\n|---|---|---|\n| **MB-E-001** | Slow server | poll skipped |\n",
        )
        .unwrap();
        git(&root, &["add", "docs"]);
        git(&root, &["commit", "-qm", "more"]);
        let sha1 = git(&root, &["rev-parse", "--short", "HEAD"]);
        git(&root, &["push", "-q", "origin", "docs/rb"]);
        git(&root, &["checkout", "-q", "main"]);
        git(&root, &["branch", "-qD", "docs/rb"]);
        let repo = Repo {
            _dir: dir,
            root,
            sha0,
            sha1,
            gh: FakeGh::new(),
        };
        repo.serve_pr("pr/rebuild.txt", None);
        repo.serve_issue(true);
        repo.gh.serve(
            &["api", "repos/{owner}/{repo}/pulls/22", "--jq", ".head.ref"],
            "docs/rb\n",
        );
        repo
    }

    fn serve_pr(&self, name: &str, sha0: Option<&str>) {
        let text = std::fs::read_to_string(fixtures().join(name))
            .unwrap()
            .replace("SHA0", sha0.unwrap_or(&self.sha0))
            .replace("SHA1", &self.sha1);
        self.gh
            .serve(&["pr", "view", "22", "--json", GH_FIELDS], &gh_json(&text));
    }

    fn serve_issue(&self, present: bool) {
        let text = std::fs::read_to_string(fixtures().join("issue/rebuild.txt")).unwrap();
        let json = if present {
            gh_json(&text)
        } else {
            String::new()
        };
        let key = ["issue", "view", "21", "--json", GH_FIELDS];
        if present {
            self.gh.clear(&key);
            self.gh.serve(&key, &json);
        } else {
            self.gh.serve_err(
                &key,
                "GraphQL: Could not resolve to an issue or pull request with the number of 21. (repository.issue)\n",
                1,
            );
        }
    }

    fn ferrule(&self, args: &[&str]) -> Run {
        self.gh
            .on(Run::new(args))
            .env("CLAUDE_PROJECT_DIR", &self.root)
            .env("FERRULE_NOW", "2026-10-05T09:00")
            .cwd(&self.root)
    }

    fn rebuild(&self) -> String {
        self.ferrule(&["run", "rebuild", "21", "22"]).text()
    }

    /// Cards and artifacts gone; the worktree stays.
    fn clear(&self) {
        for id in ["rb.s2", "rb.s1", "rb.s0"] {
            self.sql(&format!("DELETE FROM cards WHERE id = '{id}'"));
        }
        self.sql("DELETE FROM cards WHERE id = 'rb'");
        let _ = std::fs::remove_dir_all(self.root.join(".ferrule/runs/rb"));
    }

    fn sql(&self, q: &str) {
        let db = self.root.join(".ferrule/state.db");
        if db.exists() {
            let c = rusqlite::Connection::open(db).unwrap();
            c.execute(q, []).unwrap();
        }
    }

    fn files(&self) -> String {
        let mut out = String::new();
        for id in ["rb", "rb.s0", "rb.s1", "rb.s2"] {
            let shown = self.ferrule(&["board", "show", id]).text();
            let _ = writeln!(
                out,
                "==> card {id} <==\n{}",
                shown.trim_end_matches("exit=0").trim_end()
            );
        }
        for f in [
            "issue.md",
            "spec-diff.md",
            "plan.summary.md",
            "architecture.summary.md",
            "architecture.md",
        ] {
            let text = std::fs::read_to_string(self.root.join(".ferrule/runs/rb").join(f)).unwrap();
            let _ = write!(out, "==> {f} <==\n{text}");
        }
        out.replace(&self.sha0, "SHA0").replace(&self.sha1, "SHA1")
    }
}

#[test]
fn it_rebuild_recreates_the_board() {
    let r = Repo::new();
    check("run-rebuild-done", &r.rebuild());
    check("run-rebuild-files", &format!("{}exit=0", r.files()));
    let list = git(&r.root, &["worktree", "list"]);
    assert!(list.contains(".ferrule/worktrees/rb "), "{list}");
    check("run-rebuild-exists", &r.rebuild());
}

#[test]
fn it_rebuild_refuses_and_writes_nothing() {
    let r = Repo::new();
    r.serve_pr("pr/ready.txt", None);
    check("run-rebuild-no-stages", &r.rebuild());
    r.serve_pr("pr/rebuild.txt", Some("deadbee"));
    check("run-rebuild-sha", &r.rebuild());
    assert!(!r.root.join(".ferrule/runs/rb").exists());
    assert_eq!(
        r.ferrule(&["board", "show", "rb"]).text(),
        "card-show=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
    r.serve_pr("pr/rebuild.txt", None);
    r.serve_issue(false);
    check("run-rebuild-no-issue", &r.rebuild());
    // A host that fails otherwise says so instead of reading as no issue.
    r.gh.serve_err(
        &["issue", "view", "21", "--json", GH_FIELDS],
        "HTTP 401: Bad credentials (https://api.github.com/graphql)\n",
        1,
    );
    let failed = r.rebuild();
    assert!(
        failed.starts_with("rebuild=refused reason=host detail=\"HTTP 401: Bad credentials"),
        "{failed}"
    );
    r.serve_issue(true);
    std::fs::create_dir_all(r.root.join(".ferrule/runs/rb")).unwrap();
    check("run-rebuild-exists-artifacts", &r.rebuild());
}

#[test]
fn it_rebuild_re_adds_a_missing_worktree_and_refuses_a_blocked_one() {
    let r = Repo::new();
    assert_eq!(r.rebuild(), "rebuild=done slug=rb stages=3 next=s2\nexit=0");
    r.clear();
    let wt = r.root.join(".ferrule/worktrees/rb");
    std::fs::remove_dir_all(&wt).unwrap();
    check("run-rebuild-stale", &r.rebuild());
    assert!(
        wt.join("docs").is_dir(),
        "a registered but missing worktree is re-added"
    );
    r.clear();
    git(
        &r.root,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    std::fs::write(&wt, "").unwrap();
    let refused = r.rebuild();
    assert!(
        refused.starts_with("rebuild=refused reason=worktree detail=\"fatal: ")
            && refused.contains("already exists\" hint=git could not add the worktree;"),
        "{refused}"
    );
    assert_eq!(
        r.ferrule(&["board", "show", "rb"]).text(),
        "card-show=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
    assert!(!r.root.join(".ferrule/runs/rb").exists());
}

#[test]
fn it_rebuild_refuses_a_ticked_stage_without_a_sha() {
    let r = Repo::new();
    let text = std::fs::read_to_string(fixtures().join("pr/rebuild.txt"))
        .unwrap()
        .replace(" (SHA0)", "")
        .replace("SHA1", &r.sha1);
    r.gh.serve(&["pr", "view", "22", "--json", GH_FIELDS], &gh_json(&text));
    let refused = r.rebuild();
    assert!(
        refused.starts_with("rebuild=refused reason=no-sha stage=s0 hint=tick the stage as"),
        "{refused}"
    );
    assert!(!r.root.join(".ferrule/runs/rb").exists());
}

#[test]
fn it_rebuild_refuses_a_nested_branch_name() {
    let r = Repo::new();
    for branch in ["feat/a/b", "feat/"] {
        r.gh.serve(
            &["api", "repos/{owner}/{repo}/pulls/22", "--jq", ".head.ref"],
            &format!("{branch}\n"),
        );
        let refused = r.rebuild();
        assert!(
            refused.starts_with(&format!(
                "rebuild=refused reason=bad-branch branch={branch} hint=name the branch"
            )),
            "{refused}"
        );
    }
    assert!(!r.root.join(".ferrule/runs/rb").exists());
}

#[test]
fn it_rebuild_refuses_a_bad_stage_before_the_worktree() {
    for (from, to, stage) in [
        ("<!-- s1 blocked", "<!-- stage blocked", "stage"),
        ("<!-- s2 blocked", "<!-- s1 blocked", "s1"),
        ("blocked-by s1 -->", "blocked-by none -->", "none"),
    ] {
        let r = Repo::new();
        let worktrees = git(&r.root, &["worktree", "list"]);
        let text = std::fs::read_to_string(fixtures().join("pr/rebuild.txt"))
            .unwrap()
            .replace("SHA0", &r.sha0)
            .replace("SHA1", &r.sha1)
            .replace(from, to);
        r.gh.serve(&["pr", "view", "22", "--json", GH_FIELDS], &gh_json(&text));
        let refused = r.rebuild();
        assert!(
            refused.starts_with(&format!(
                "rebuild=refused reason=bad-stage stage={stage} hint=mark each"
            )),
            "{refused}"
        );
        assert_eq!(git(&r.root, &["worktree", "list"]), worktrees);
        assert!(!r.root.join(".ferrule/runs/rb").exists());
    }
}
