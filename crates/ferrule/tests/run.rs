mod golden;

use std::path::{Path, PathBuf};
use std::process::Command;

use golden::Run;

const NOW: &str = "2026-10-05T09:00";

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

/// A main checkout with run `ss`: its parent card in inprogress, s0 done,
/// `stages` more stages open, its worktree on `feat/ss`, and a plan summary
/// asking for verification beyond unit tests on s3.
struct Scratch {
    _dir: tempfile::TempDir,
    root: PathBuf,
    wt: PathBuf,
}

impl Scratch {
    fn new(stages: u32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("s");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join(".gitignore"), ".ferrule/\n").unwrap();
        git(&root, &["add", ".gitignore"]);
        git(&root, &["commit", "-q", "-m", "init"]);
        let wt = root.join(".ferrule/worktrees/ss");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                wt.to_str().unwrap(),
                "-b",
                "feat/ss",
                "main",
            ],
        );
        let s = Scratch {
            _dir: dir,
            root,
            wt,
        };
        s.ok(&[
            "card",
            "new",
            "ss",
            "--state",
            "inprogress",
            "branch=feat/ss",
            "lane=full",
            "gate1=2026-10-01",
            "arch=2026-10-02",
            "gate2=2026-10-03",
        ]);
        for n in 0..=stages {
            let id = format!("ss.s{n}");
            let state = if n == 0 { "done" } else { "open" };
            let title = format!("title=Stage {n}");
            let blocked = if n == 0 {
                "blocked-by=[]".to_owned()
            } else {
                format!("blocked-by=[ss.s{}]", n - 1)
            };
            let files = format!("files=[f{n}.txt]");
            s.ok(&[
                "card", "new", &id, "--state", state, &title, &blocked, &files,
            ]);
        }
        s.artifact(
            "plan.summary.md",
            "# ss — plan summary (rev 1)\n\nDecide: x\n\nVerification beyond unit tests:\n- s3 — demo TUI\n  - see it\n",
        );
        s
    }

    fn run(&self, args: &[&str]) -> Run {
        Run::new(args)
            .env("CLAUDE_PROJECT_DIR", &self.root)
            .env("FERRULE_NOW", NOW)
            .cwd(&self.root)
    }

    /// Stdout, the trailing newline dropped.
    fn say(&self, args: &[&str]) -> String {
        let out = self.run(args).output();
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    }

    fn ok(&self, args: &[&str]) {
        assert_eq!(self.run(args).text(), "exit=0", "{args:?}");
    }

    fn artifact(&self, name: &str, text: &str) {
        let dir = self.root.join(".ferrule/runs/ss");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn remove_artifact(&self, name: &str) {
        std::fs::remove_file(self.root.join(".ferrule/runs/ss").join(name)).unwrap();
    }

    fn config(&self, toml: &str) {
        std::fs::write(self.root.join(".ferrule/ferrule.toml"), toml).unwrap();
    }

    fn last_line(&self, card: &str) -> String {
        self.say(&["card", "show", card])
            .lines()
            .last()
            .unwrap_or_default()
            .to_owned()
    }

    fn commit_in_worktree(&self, file: &str, text: &str, msg: &str) -> String {
        std::fs::write(self.wt.join(file), text).unwrap();
        git(&self.wt, &["add", file]);
        git(&self.wt, &["commit", "-q", "-m", msg]);
        git(&self.wt, &["rev-parse", "--short", "HEAD"])
    }
}

#[test]
fn it_stage_stop_rules_in_order() {
    let s = Scratch::new(5);
    let stop = |sid: &str| s.say(&["run", "stage-stop", "ss", sid]);
    assert_eq!(stop("s0"), "auto");
    assert_eq!(stop("s1"), "auto");
    assert_eq!(s.last_line("ss.s1"), "2026-10-05T09:00 auto=yes");
    s.artifact(
        "review.verdict.md",
        "Verdict: findings — 0 must fix, 1 needs you\n\nNeeds user\n- keep it?\n  - a) yes\n",
    );
    assert_eq!(stop("s1"), "stop=1 reason=needs-user");
    assert_eq!(s.last_line("ss.s1"), "2026-10-05T09:00 stop=1");
    s.artifact(
        "review.verdict.md",
        "Verdict: clean — ready for approval\n\nNeeds user\n- none\n",
    );
    assert_eq!(stop("s3"), "stop=2 reason=verification detail=\"demo TUI\"");
    assert_eq!(
        s.say(&["run", "stage-stop", "ss", "s1", "--threads", "2"]),
        "stop=3 reason=pr-threads detail=\"2 threads\""
    );
    assert_eq!(stop("s5"), "auto");
    s.ok(&["card", "set", "ss.s2", "stop=yes"]);
    assert_eq!(stop("s2"), "stop=5 reason=plan-marked");
    s.ok(&["card", "set", "ss.s5", "stop=yes"]);
    assert_eq!(stop("s5"), "stop=5 reason=plan-marked");

    std::fs::write(s.wt.join("a.txt"), "a\n").unwrap();
    std::fs::write(s.wt.join("b.txt"), "b\nc\n").unwrap();
    s.config("[stage_stop]\nfiles = 1\n");
    assert_eq!(
        stop("s1"),
        "stop=6 reason=diff-size detail=\"3 lines, 2 files\""
    );
    s.config("[stage_stop]\nfiles = 2\n");
    assert_eq!(stop("s1"), "auto");
    assert_eq!(stop("s9"), "stage-stop=card");
}

#[test]
fn it_stage_stop_checkpoint_after_three_autos() {
    let s = Scratch::new(5);
    s.remove_artifact("plan.summary.md");
    for n in 1..=3 {
        assert_eq!(
            s.say(&["run", "stage-stop", "ss", &format!("s{n}")]),
            "auto"
        );
    }
    assert_eq!(
        s.say(&["run", "stage-stop", "ss", "s4"]),
        "stop=7 reason=checkpoint"
    );

    let s = Scratch::new(5);
    s.ok(&["card", "log", "ss.s2", "stop=5"]);
    s.ok(&["card", "log", "ss.s3", "auto=yes"]);
    assert_eq!(s.say(&["run", "stage-stop", "ss", "s4"]), "auto");
}

#[test]
fn it_sha_check_and_reconcile() {
    let s = Scratch::new(2);
    let s1 = s.commit_in_worktree("f1.txt", "x\n", "s1");
    s.ok(&["card", "log", "ss.s1", &format!("commit={s1}")]);
    let check = |sha: &str| s.run(&["run", "sha-check", "ss", "s1", sha]).text();
    assert_eq!(check(&s1), "sha=ok\nexit=0");
    assert_eq!(check("deadbee"), "sha=mismatch reason=missing\nexit=1");
    git(&s.root, &["commit", "-q", "--allow-empty", "-m", "on-main"]);
    let m = git(&s.root, &["rev-parse", "--short", "HEAD"]);
    assert_eq!(check(&m), "sha=mismatch reason=off-branch\nexit=1");
    let s1b = s.commit_in_worktree("f1.txt", "x\ny\n", "s1b");
    assert_eq!(check(&s1b), "sha=mismatch reason=card-commit\nexit=1");
    assert_eq!(
        s.run(&["run", "sha-check", "ss", "s7", &s1]).text(),
        "sha=mismatch reason=card\nexit=1"
    );

    s.ok(&["card", "log", "ss.s0", &format!("commit={s1}")]);
    assert_eq!(
        s.run(&["run", "reconcile", "ss"]).text(),
        "reconcile=clean next=s1\nexit=0"
    );
    s.ok(&["card", "log", "ss.s2", "commit=deadbee"]);
    s.ok(&["card", "log", "ss", "gauntlet=pass", &format!("sha={m}")]);
    assert_eq!(
        s.run(&["run", "reconcile", "ss"]).text(),
        format!(
            "differ row=commit s2 deadbee does not exist\ndiffer row=gauntlet {m} not on feat/ss\nreconcile=differ n=2 next=s1\nexit=1"
        )
    );
    git(
        &s.root,
        &["worktree", "remove", "--force", s.wt.to_str().unwrap()],
    );
    let first = s.say(&["run", "reconcile", "ss"]);
    assert_eq!(
        first.lines().next().unwrap(),
        format!("differ row=worktree {} missing", s.wt.display())
    );
    assert_eq!(s.run(&["run", "reconcile", "nosuch"]).text(), "exit=2");
}
