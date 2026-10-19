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
    s.config("[thresholds]\nstage_stop_files = 1\n");
    assert_eq!(
        stop("s1"),
        "stop=6 reason=diff-size detail=\"3 lines, 2 files\""
    );
    s.config("[thresholds]\nstage_stop_files = 2\n");
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

/// Run `demo` from the fixture board with its worktree pushed to a bare
/// remote, and its artifacts in `.ferrule/runs/demo/`.
fn pushed_run() -> Scratch {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("r");
    let remote = dir.path().join("remote.git");
    std::fs::create_dir(&root).unwrap();
    git(
        dir.path(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), ".ferrule/\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-q", "-m", "init"]);
    git(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&root, &["push", "-q", "origin", "main"]);
    let wt = root.join(".ferrule/worktrees/demo");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().unwrap(),
            "-b",
            "feat/demo",
            "main",
        ],
    );
    std::fs::write(wt.join("x.txt"), "x\n").unwrap();
    git(&wt, &["add", "x.txt"]);
    git(&wt, &["commit", "-q", "-m", "stage"]);
    git(&wt, &["push", "-q", "-u", "origin", "feat/demo"]);
    let s = Scratch {
        _dir: dir,
        root,
        wt,
    };
    let board = golden::fixtures().join("board");
    assert_eq!(
        s.say(&["card", "import", board.to_str().unwrap()]),
        "import=ok cards=13"
    );
    for slug in ["demo", "nospec"] {
        let from = board.join("artifacts").join(slug);
        let to = s.root.join(".ferrule/runs").join(slug);
        std::fs::create_dir_all(&to).unwrap();
        for f in std::fs::read_dir(&from).unwrap() {
            let f = f.unwrap().path();
            std::fs::copy(&f, to.join(f.file_name().unwrap())).unwrap();
        }
    }
    s
}

#[test]
fn it_cleanup_refuses_until_pushed_and_green_then_archives() {
    let s = pushed_run();
    let cleanup = || s.say(&["run", "cleanup", "demo"]);
    std::fs::write(s.wt.join("y.txt"), "y\n").unwrap();
    assert_eq!(cleanup(), "cleanup=refused reason=uncommitted-changes");
    git(&s.wt, &["add", "y.txt"]);
    git(&s.wt, &["commit", "-q", "-m", "more"]);
    assert_eq!(cleanup(), "cleanup=refused reason=unpushed-commits");
    git(&s.wt, &["push", "-q"]);
    assert_eq!(cleanup(), "cleanup=refused reason=no-ci");
    assert!(s.wt.is_dir());
    s.ok(&["card", "log", "demo", "ci=1"]);
    assert_eq!(cleanup(), "cleanup=refused reason=ci-red");
    assert!(s.wt.is_dir());
    assert_eq!(s.say(&["run", "metrics"]), "");

    s.ok(&["card", "log", "demo", "ci=0"]);
    assert_eq!(cleanup(), "cleanup=done archive=1");
    assert!(!s.wt.exists());
    assert!(!has_ref(&s.root, "refs/heads/feat/demo"));
    assert!(!s.root.join(".ferrule/runs/demo").exists());
    assert!(s.root.join(".ferrule/runs/nospec/issue.md").exists());
    assert_eq!(s.say(&["card", "field", "nospec", "state"]), "inreview");
    assert_eq!(s.say(&["card", "field", "demo", "state"]), "");
    let metrics = s.say(&["run", "metrics"]);
    assert!(
        metrics.starts_with("demo start=") && metrics.ends_with(" ci=0"),
        "{metrics}"
    );
    assert_eq!(cleanup(), "cleanup=done archive=none");

    let to = s.root.join("out");
    assert_eq!(
        s.say(&["run", "export", "demo", "--to", to.to_str().unwrap()]),
        format!("export=done archive=1 path={}", to.display())
    );
    let card = std::fs::read_to_string(to.join("cards/demo.s1.md")).unwrap();
    assert!(
        card.starts_with("---\nid: demo.s1\nparent: demo\nstate: inreview\n"),
        "{card}"
    );
    let original = golden::fixtures().join("board/artifacts/demo/spec-diff.md");
    assert_eq!(
        std::fs::read(to.join("artifacts/spec-diff.md")).unwrap(),
        std::fs::read(original).unwrap()
    );
    assert_eq!(s.say(&["run", "export", "nospec"]), "export=none");
}

#[test]
fn it_cleanup_of_an_epic_needs_no_ci() {
    let s = pushed_run();
    assert_eq!(s.say(&["run", "cleanup", "lb"]), "cleanup=done archive=1");
    assert_eq!(
        s.say(&["run", "cleanup", "lb"]),
        "cleanup=done archive=none"
    );
}

fn has_ref(dir: &Path, r: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "-q", "--verify", r])
        .output()
        .unwrap()
        .status
        .success()
}

const STEPS: &str = r#"
[[gauntlet.step]]
name = "fmt"
cmd = ["sh", "-c", "echo fmt-out"]

[[gauntlet.step]]
name = "cov"
cmd = ["sh", "-c", "echo 'TOTAL 100 20 80.00% 10 1 90.00% 500 94 81.20%'"]
coverage = "llvm-cov"
"#;

#[test]
fn it_gauntlet_runs_the_configured_steps_and_caches_a_pass() {
    let s = Scratch::new(1);
    let g = |extra: &[&str]| {
        let mut args = vec!["run", "gauntlet", "ss"];
        args.extend_from_slice(extra);
        s.run(&args).text()
    };
    assert_eq!(g(&[]), "gauntlet=fail step=config reason=no-steps\nexit=1");
    s.config(STEPS);
    let sha = git(&s.wt, &["rev-parse", "--short", "HEAD"]);
    let pass = format!("gauntlet=pass cov=81.20% sha={sha}");
    assert_eq!(g(&[]), format!("{pass}\nexit=0"));
    let log = std::fs::read_to_string(s.root.join(".ferrule/runs/ss/gauntlet.log")).unwrap();
    assert!(
        log.starts_with("### fmt: sh -c echo fmt-out\nfmt-out\n### cov:"),
        "{log}"
    );
    assert_eq!(g(&[]), format!("{pass} cached\nexit=0"));
    assert_eq!(g(&["--force"]), format!("{pass}\nexit=0"));

    std::fs::write(s.wt.join("b.txt"), "b\n").unwrap();
    assert_eq!(
        g(&[]),
        format!("{pass}\nexit=0"),
        "an untracked file misses the cache"
    );
    assert_eq!(git(&s.wt, &["status", "--porcelain"]), "?? b.txt");

    s.config(&format!(
        "{STEPS}\n[[gauntlet.step]]\nname = \"lint\"\ncmd = [\"false\"]\n"
    ));
    let log = s.root.join(".ferrule/runs/ss/gauntlet.log");
    assert_eq!(
        g(&["--force"]),
        format!(
            "gauntlet=fail step=lint sha={sha} log={}\nexit=1",
            log.display()
        )
    );
    s.config("[[gauntlet.step]]\nname = \"test\"\ncmd = [\"sleep\", \"5\"]\ntimeout = \"1s\"\n");
    assert_eq!(
        g(&["--force"]),
        format!(
            "gauntlet=fail step=test sha={sha} reason=timeout log={}\nexit=1",
            log.display()
        )
    );
    std::fs::remove_dir_all(&s.wt).unwrap();
    assert_eq!(g(&[]), "gauntlet=fail step=cd reason=no-worktree\nexit=1");
}
