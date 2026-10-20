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
            "board",
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
                "board", "new", &id, "--state", state, &title, &blocked, &files,
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
        self.say(&["board", "show", card])
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
    let stop = |sid: &str| s.say(&["stage", "stop", "ss", sid]);
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
        s.say(&["stage", "stop", "ss", "s1", "--threads", "2"]),
        "stop=3 reason=pr-threads detail=\"2 threads\""
    );
    assert_eq!(stop("s5"), "auto");
    s.ok(&["board", "set", "ss.s2", "stop=yes"]);
    assert_eq!(stop("s2"), "stop=5 reason=plan-marked");
    s.ok(&["board", "set", "ss.s5", "stop=yes"]);
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
    assert!(stop("s9").starts_with("stage-stop=card hint="));
}

#[test]
fn it_stage_stop_checkpoint_after_three_autos() {
    let s = Scratch::new(5);
    s.remove_artifact("plan.summary.md");
    for n in 1..=3 {
        assert_eq!(s.say(&["stage", "stop", "ss", &format!("s{n}")]), "auto");
    }
    assert_eq!(
        s.say(&["stage", "stop", "ss", "s4"]),
        "stop=7 reason=checkpoint"
    );

    let s = Scratch::new(5);
    s.ok(&["board", "log", "ss.s2", "stop=5"]);
    s.ok(&["board", "log", "ss.s3", "auto=yes"]);
    assert_eq!(s.say(&["stage", "stop", "ss", "s4"]), "auto");
}

#[test]
fn it_sha_check_and_reconcile() {
    let s = Scratch::new(2);
    let s1 = s.commit_in_worktree("f1.txt", "x\n", "s1");
    s.ok(&["board", "log", "ss.s1", &format!("commit={s1}")]);
    let check = |sha: &str| s.run(&["stage", "verify", "ss", "s1", sha]).text();
    assert_eq!(check(&s1), "sha=ok\nexit=0");
    assert!(check("deadbee").starts_with("sha=mismatch reason=missing hint="));
    git(&s.root, &["commit", "-q", "--allow-empty", "-m", "on-main"]);
    let m = git(&s.root, &["rev-parse", "--short", "HEAD"]);
    assert!(check(&m).starts_with("sha=mismatch reason=off-branch hint="));
    let s1b = s.commit_in_worktree("f1.txt", "x\ny\n", "s1b");
    assert!(check(&s1b).starts_with("sha=mismatch reason=card-commit hint="));
    assert!(
        s.run(&["stage", "verify", "ss", "s7", &s1])
            .text()
            .starts_with("sha=mismatch reason=card hint=")
    );

    s.ok(&["board", "log", "ss.s0", &format!("commit={s1}")]);
    assert_eq!(
        s.run(&["run", "reconcile", "ss"]).text(),
        "reconcile=clean next=s1\nexit=0"
    );
    s.ok(&["board", "log", "ss.s2", "commit=deadbee"]);
    s.ok(&["board", "log", "ss", "gauntlet=pass", &format!("sha={m}")]);
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
        s.say(&["board", "import", board.to_str().unwrap()]),
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
    assert!(cleanup().starts_with("cleanup=refused reason=uncommitted-changes hint="));
    git(&s.wt, &["add", "y.txt"]);
    git(&s.wt, &["commit", "-q", "-m", "more"]);
    assert!(cleanup().starts_with("cleanup=refused reason=unpushed-commits hint="));
    git(&s.wt, &["push", "-q"]);
    assert!(cleanup().starts_with("cleanup=refused reason=no-ci hint="));
    assert!(s.wt.is_dir());
    s.ok(&["board", "log", "demo", "ci=1"]);
    assert!(cleanup().starts_with("cleanup=refused reason=ci-red hint="));
    assert!(s.wt.is_dir());
    assert_eq!(s.say(&["run", "metrics"]), "");

    s.ok(&["board", "log", "demo", "ci=0"]);
    assert_eq!(cleanup(), "cleanup=done archive=1");
    assert!(!s.wt.exists());
    assert!(!has_ref(&s.root, "refs/heads/feat/demo"));
    assert!(!s.root.join(".ferrule/runs/demo").exists());
    assert!(s.root.join(".ferrule/runs/nospec/issue.md").exists());
    assert_eq!(s.say(&["board", "field", "nospec", "state"]), "inreview");
    assert_eq!(s.say(&["board", "field", "demo", "state"]), "");
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
    assert!(
        s.say(&["run", "export", "nospec"])
            .starts_with("export=none hint=")
    );
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
        let mut args = vec!["ci", "gauntlet", "ss"];
        args.extend_from_slice(extra);
        s.run(&args).text()
    };
    assert!(g(&[]).starts_with("gauntlet=fail step=config reason=no-steps hint="));
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
    assert!(g(&[]).starts_with("gauntlet=fail step=cd reason=no-worktree hint="));
}

#[test]
fn it_run_diff_and_log_read_the_branch_against_main() {
    let s = Scratch::new(1);
    s.commit_in_worktree("f1.txt", "one\n", "add f1");
    std::fs::create_dir_all(s.wt.join("docs/specs")).unwrap();
    s.commit_in_worktree("docs/specs/a.md", "**MB-R-002** — Polls.\n", "spec");
    let stat = s.say(&["run", "diff", "ss", "--stat"]);
    assert!(
        stat.contains("f1.txt") && stat.contains("docs/specs/a.md"),
        "{stat}"
    );
    let specs = s.say(&["run", "diff", "ss", "--specs"]);
    assert!(specs.contains("+**MB-R-002** — Polls."), "{specs}");
    assert!(!specs.contains("f1.txt"), "{specs}");
    let log = s.say(&["run", "log", "ss"]);
    assert_eq!(log.lines().count(), 2, "{log}");
    assert!(log.lines().next().unwrap().ends_with(" spec"), "{log}");
    std::fs::write(s.wt.join("f1.txt"), "changed\n").unwrap();
    let wip = s.say(&["run", "diff", "ss", "--uncommitted"]);
    assert!(
        wip.starts_with(" M f1.txt\n") && wip.contains("+changed"),
        "{wip}"
    );
    assert_eq!(
        s.run(&["run", "diff", "ss", "--base", "nosuch"]).text(),
        "diff=refused reason=base hint=fetch it, or pass a --base the branch has\nexit=1"
    );
    assert_eq!(
        s.run(&["run", "log", "zz"]).text(),
        "log=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
    // --uncommitted needs no base, so a main_branch the worktree lacks is fine
    s.config("main_branch = \"trunk\"\n");
    assert_eq!(
        s.run(&["run", "diff", "ss"]).text(),
        "diff=refused reason=base hint=fetch it, or pass a --base the branch has\nexit=1"
    );
    let wip = s.say(&["run", "diff", "ss", "--uncommitted"]);
    assert!(wip.starts_with(" M f1.txt\n"), "{wip}");
}

#[test]
fn it_plan_diff_compares_the_snapshot() {
    let s = Scratch::new(1);
    assert_eq!(
        s.run(&["run", "plan-diff", "ss"]).text(),
        "plan-diff=refused reason=no-plan hint=the planner has not written plan.md yet\nexit=1"
    );
    s.artifact("plan.md", "# Plan\n\nStep one.\n");
    assert!(
        s.say(&["run", "plan-diff", "ss"])
            .starts_with("plan-diff=refused reason=no-prev")
    );
    s.artifact("plan.prev.md", "# Plan\n\nStep one.\n");
    assert_eq!(
        s.run(&["run", "plan-diff", "ss"]).text(),
        "plan-diff=same\nexit=0"
    );
    s.artifact("plan.md", "# Plan\n\nStep two.\n");
    let d = s.say(&["run", "plan-diff", "ss"]);
    assert!(d.contains("-Step one.\n+Step two."), "{d}");
}

#[test]
fn it_spec_next_counts_main_and_with_run_the_run() {
    let s = Scratch::new(1);
    std::fs::create_dir_all(s.root.join("docs/specs")).unwrap();
    std::fs::write(
        s.root.join("docs/specs/a.md"),
        "**MB-R-007** — x.\nSee MB-R-009. **MB-E-002** — y. XMB-R-500 MB-R-501x\n",
    )
    .unwrap();
    git(&s.root, &["add", "docs"]);
    git(&s.root, &["commit", "-q", "-m", "specs"]);
    assert_eq!(
        s.say(&["spec", "next", "MB"]),
        "next=MB-R-010 last=MB-R-009"
    );
    assert_eq!(
        s.say(&["spec", "next", "MB", "--edge", "--count", "2"]),
        "next=MB-E-003,MB-E-004 last=MB-E-002"
    );
    assert_eq!(s.say(&["spec", "next", "TR"]), "next=TR-R-001 last=-");
    s.artifact("spec-diff.md", "## MB-R-012 — new\n");
    s.ok(&[
        "board",
        "log",
        "ss.s1",
        "spec-delta",
        "MB-R-015",
        "added",
        "origin=impl",
    ]);
    assert_eq!(
        s.say(&["spec", "next", "MB"]),
        "next=MB-R-010 last=MB-R-009"
    );
    assert_eq!(
        s.say(&["spec", "next", "MB", "--run", "ss"]),
        "next=MB-R-016 last=MB-R-015"
    );
    assert_eq!(
        s.run(&["spec", "next", "MB", "--count", "4294967295"])
            .text(),
        "next=refused reason=range hint=lower --count; the numbers would pass 4294967295\nexit=1"
    );
    assert_eq!(
        s.run(&["spec", "next", "mb"]).text(),
        "next=refused reason=prefix hint=a prefix is two capital letters, e.g. MB\nexit=1"
    );
}

#[test]
fn it_spec_refs_finds_whole_ids() {
    let s = Scratch::new(1);
    std::fs::write(
        s.wt.join("lib.rs"),
        "// MB-R-001\n// MB-R-0010\nfn f() {} // MB-R-002\n",
    )
    .unwrap();
    git(&s.wt, &["add", "lib.rs"]);
    git(&s.wt, &["commit", "-q", "-m", "lib"]);
    let got = Run::new(["spec", "refs", "MB-R-001", "MB-R-002"])
        .env("CLAUDE_PROJECT_DIR", &s.wt)
        .cwd(&s.wt)
        .text();
    assert_eq!(
        got,
        "lib.rs:1:// MB-R-001\nlib.rs:3:fn f() {} // MB-R-002\nexit=0"
    );
    assert_eq!(
        s.run(&["spec", "refs", "MB-R-001"]).text(),
        "refs=none\nexit=1"
    );
    let on_branch = s.say(&[
        "spec", "refs", "MB-R-002", "--ref", "feat/ss", "--path", "lib.rs",
    ]);
    assert_eq!(on_branch, "lib.rs:3:fn f() {} // MB-R-002");
    assert_eq!(
        s.run(&["spec", "refs", "MB-R-001", "--path", ":(nosuchmagic)x"])
            .text(),
        "refs=refused reason=grep hint=git grep failed; check each --path, else report it to the user\nexit=1"
    );
    assert_eq!(
        s.run(&["spec", "refs", "R1"]).text(),
        "refs=refused reason=id id=R1 hint=a fixed ID looks like MB-R-001\nexit=2"
    );
}

#[test]
fn it_board_events_reads_the_whole_run() {
    let s = Scratch::new(2);
    s.ok(&["board", "log", "ss.s1", "follow-up", "--", "rename x"]);
    s.ok(&["board", "log", "ss.s2", "follow-up", "--", "drop y"]);
    let got = s.say(&["board", "events", "ss", "--event", "follow-up"]);
    assert_eq!(
        got,
        format!("ss.s1 {NOW} follow-up rename x\nss.s2 {NOW} follow-up drop y")
    );
    assert_eq!(
        s.say(&[
            "board",
            "events",
            "ss",
            "--event",
            "follow-up",
            "--last",
            "1"
        ]),
        format!("ss.s2 {NOW} follow-up drop y")
    );
    assert_eq!(
        s.run(&["board", "events", "zz"]).text(),
        "events=card hint=check the id with `ferrule board list`\nexit=1"
    );
}

#[test]
fn it_run_status_summarises_cards_and_worktree() {
    let s = Scratch::new(1);
    let sha = s.commit_in_worktree("f1.txt", "one\n", "add f1");
    std::fs::write(s.wt.join("f1.txt"), "two\n").unwrap();
    s.ok(&["board", "log", "ss.s1", "note", "--", "started"]);
    let got = s.say(&["run", "status", "ss"]);
    let lines: Vec<&str> = got.lines().collect();
    assert_eq!(
        lines[0],
        "run=ss state=inprogress lane=full branch=feat/ss issue=- pr=- pr-state=- gates=gate1,arch,gate2"
    );
    assert_eq!(lines[1], "stage=s0 state=done commit=- title=Stage 0");
    assert_eq!(lines[2], "stage=s1 state=open commit=- title=Stage 1");
    assert_eq!(
        lines[3],
        format!("worktree=.ferrule/worktrees/ss head={sha} ahead=1 uncommitted=1")
    );
    assert_eq!(lines[4..6], ["gauntlet=-", "ci=-"]);
    assert_eq!(lines[6], format!("last=ss.s1 {NOW} note started"));
    s.ok(&["board", "log", "ss", "pr=draft"]);
    s.ok(&["board", "log", "ss", "gauntlet=pass", "cov=80%", "sha=abc"]);
    s.ok(&["board", "log", "ss", "ci=0"]);
    s.ok(&["board", "log", "ss", "note", "--", "see pr=7 and ci=1"]);
    let got = s.say(&["run", "status", "ss"]);
    let lines: Vec<&str> = got.lines().collect();
    assert!(lines[0].contains(" pr-state=draft "), "{got}");
    assert_eq!(lines[4..6], ["gauntlet=pass cov=80% sha=abc", "ci=0"]);
    git(
        &s.root,
        &["worktree", "remove", "--force", s.wt.to_str().unwrap()],
    );
    let got = s.say(&["run", "status", "ss"]);
    assert_eq!(
        got.lines().nth(3),
        Some("worktree=missing path=.ferrule/worktrees/ss")
    );
    assert_eq!(
        s.run(&["run", "status", "zz"]).text(),
        "status=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
}
