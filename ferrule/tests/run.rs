mod golden;

use std::path::{Path, PathBuf};
use std::process::Command;

use golden::{FakeGh, GH_FIELDS, Run, gh_json, seed_card, seed_log, seed_set};

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
        seed_card(
            &s.root,
            "ss",
            "inprogress",
            &[
                "branch=feat/ss",
                "lane=full",
                "gate1=2026-10-01",
                "arch=2026-10-02",
                "gate2=2026-10-03",
            ],
        );
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
            seed_card(&s.root, &id, state, &[&title, &blocked, &files]);
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

/// GA-R-102, GA-R-103, GA-E-016 — the rules fire in order, a plan-marked
/// stage stops on a clean verdict, and a large diff does not stop.
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

    for i in 0..9 {
        let text = if i == 0 {
            "x\n".repeat(500)
        } else {
            "x\n".to_owned()
        };
        std::fs::write(s.wt.join(format!("big{i}.txt")), text).unwrap();
    }
    assert_eq!(stop("s1"), "auto");
    assert!(stop("s9").starts_with("stage-stop=refused reason=card hint="));
}

/// A run whose stage `n` carries the `n`th of `tiers`.
fn tiered(tiers: &[&str]) -> Scratch {
    let s = Scratch::new(u32::try_from(tiers.len()).unwrap());
    for (i, t) in tiers.iter().enumerate() {
        seed_set(&s.root, &format!("ss.s{}", i + 1), &[&format!("tier={t}")]);
    }
    s
}

fn tier_of(s: &Scratch, n: u32) -> String {
    s.say(&["stage", "tier", "ss", &format!("s{n}")])
}

/// Commits `s<n>`'s file in the worktree and logs the commit on its card.
fn land_stage(s: &Scratch, n: u32, text: &str) {
    let sha = s.commit_in_worktree(&format!("f{n}.txt"), text, &format!("s{n}"));
    seed_log(
        &s.root,
        NOW,
        &format!("ss.s{n}"),
        &[&format!("commit={sha}")],
    );
}

fn lines_of(n: usize) -> String {
    "x\n".repeat(n)
}

/// GA-R-086 — a batch of two closes at its second stage, before the
/// own stage; its first stage has no review point.
#[test]
fn it_stage_tier_batch_of_two_closes_at_the_second() {
    let s = tiered(&["batched", "batched", "own"]);
    s.commit_in_worktree("f1.txt", "a\n", "s1");
    assert_eq!(tier_of(&s, 1), "tier=batched review=none");
    let sha = git(&s.wt, &["rev-parse", "--short", "HEAD"]);
    seed_log(&s.root, NOW, "ss.s1", &[&format!("commit={sha}")]);
    std::fs::write(s.wt.join("f2.txt"), "b\n").unwrap();
    assert_eq!(tier_of(&s, 2), "tier=batched review=batch stages=s1,s2");
    assert_eq!(tier_of(&s, 3), "tier=own review=stage");
}

/// A skipped stage has no review point.
#[test]
fn it_stage_tier_skipped_stage_has_no_review() {
    let s = tiered(&["skipped"]);
    assert_eq!(tier_of(&s, 1), "tier=skipped review=none");
    assert!(s.last_line("ss.s1").ends_with("tier=skipped"));
}

/// GA-R-074 — a batch closes at three stages.
#[test]
fn it_stage_tier_batch_closes_at_three_stages() {
    let s = tiered(&["batched", "batched", "batched", "batched"]);
    land_stage(&s, 1, "a\n");
    land_stage(&s, 2, "b\n");
    assert_eq!(tier_of(&s, 2), "tier=batched review=none");
    std::fs::write(s.wt.join("f3.txt"), "c\n").unwrap();
    assert_eq!(tier_of(&s, 3), "tier=batched review=batch stages=s1,s2,s3");
}

/// GA-R-074, GA-E-013 — the stage that reaches 1,200 changed lines closes
/// its batch and is the last it covers; a lone stage over the cap covers itself.
#[test]
fn it_stage_tier_crossing_1200_lines_closes_the_batch() {
    let s = tiered(&["batched", "batched", "batched"]);
    land_stage(&s, 1, &lines_of(700));
    std::fs::write(s.wt.join("f2.txt"), lines_of(500)).unwrap();
    assert_eq!(tier_of(&s, 2), "tier=batched review=batch stages=s1,s2");
    let s = tiered(&["batched", "batched"]);
    std::fs::write(s.wt.join("f1.txt"), lines_of(1200)).unwrap();
    assert_eq!(tier_of(&s, 1), "tier=batched review=batch stages=s1");
    let s = tiered(&["batched", "batched"]);
    std::fs::write(s.wt.join("f1.txt"), lines_of(1199)).unwrap();
    assert_eq!(tier_of(&s, 1), "tier=batched review=none");
}

/// GA-R-087 — a batch open at the last stage closes there.
#[test]
fn it_stage_tier_batch_closes_at_the_last_stage() {
    let s = tiered(&["batched", "batched"]);
    land_stage(&s, 1, "a\n");
    assert_eq!(tier_of(&s, 1), "tier=batched review=none");
    std::fs::write(s.wt.join("f2.txt"), "b\n").unwrap();
    assert_eq!(tier_of(&s, 2), "tier=batched review=batch stages=s1,s2");
}

/// GA-R-072 — a planned batched stage that changes the spec is promoted to
/// its own review, logged on its card.
#[test]
fn it_stage_tier_spec_change_promotes_the_stage() {
    let s = tiered(&["batched", "batched"]);
    std::fs::create_dir_all(s.wt.join("docs/specs")).unwrap();
    std::fs::write(s.wt.join("docs/specs/x.md"), "x\n").unwrap();
    assert_eq!(tier_of(&s, 1), "tier=own review=stage promoted=spec-change");
    assert!(
        s.last_line("ss.s1")
            .ends_with("tier=own promoted=spec-change")
    );
    assert_eq!(tier_of(&s, 1), "tier=own review=stage promoted=spec-change");
}

/// GA-E-008 — a stage promoted by a spec change closes the open batch, and
/// one review covers the batch and the promoted stage.
#[test]
fn it_stage_tier_promoted_stage_closes_the_open_batch() {
    let s = tiered(&["batched", "batched", "batched"]);
    land_stage(&s, 1, "a\n");
    land_stage(&s, 2, "b\n");
    assert_eq!(tier_of(&s, 2), "tier=batched review=none");
    std::fs::create_dir_all(s.wt.join("docs/specs")).unwrap();
    std::fs::write(s.wt.join("docs/specs/x.md"), "x\n").unwrap();
    assert_eq!(
        tier_of(&s, 3),
        "tier=own review=batch stages=s1,s2,s3 promoted=spec-change"
    );
}

/// GA-E-012 — stage 0 lands the spec text and keeps its tier.
#[test]
fn it_stage_tier_s0_keeps_its_tier() {
    let s = tiered(&["own"]);
    seed_set(&s.root, "ss.s0", &["tier=batched"]);
    std::fs::create_dir_all(s.wt.join("docs/specs")).unwrap();
    std::fs::write(s.wt.join("docs/specs/x.md"), "x\n").unwrap();
    let out = tier_of(&s, 0);
    assert!(out.starts_with("tier=batched review="), "{out}");
    assert!(!out.contains("promoted"), "{out}");
}

/// GA-R-073 — after a batch blocker the next stage gets its own review.
#[test]
fn it_stage_tier_after_blocker_gives_own_review() {
    let s = tiered(&["batched", "batched"]);
    assert_eq!(
        s.say(&["stage", "tier", "ss", "s1", "--after-blocker"]),
        "tier=own review=stage promoted=batch-blocker"
    );
    assert_eq!(
        tier_of(&s, 1),
        "tier=own review=stage promoted=batch-blocker"
    );
    assert!(
        s.last_line("ss.s1")
            .ends_with("tier=own promoted=batch-blocker")
    );
}

/// GA-E-007 — a rerun after the batch's earlier stage committed, and after
/// the last one did too, names the same stages.
#[test]
fn it_stage_tier_mid_batch_rerun_keeps_committed_stages() {
    let s = tiered(&["batched", "batched", "own"]);
    land_stage(&s, 1, "a\n");
    std::fs::write(s.wt.join("f2.txt"), "b\n").unwrap();
    let want = "tier=batched review=batch stages=s1,s2";
    assert_eq!(tier_of(&s, 2), want);
    land_stage(&s, 2, "b\n");
    assert_eq!(tier_of(&s, 2), want);
    assert_eq!(tier_of(&s, 2), want);
    assert!(tier_of(&s, 9).starts_with("stage-tier=refused reason=card hint="));
}

/// GA-R-091 — at a review point `{crates}` takes the packages the batch's
/// stages touched since its base.
#[test]
fn it_gauntlet_review_level_takes_crates_from_the_batch_base() {
    let s = tiered(&["batched", "batched"]);
    s.config(
        "stack = \"rust\"\n[[gauntlet.step]]\nname = \"crates\"\ncmd = [\"echo\", \"{crates}\"]\nat = [\"review\"]\n",
    );
    let mut base = String::new();
    for (dir, name) in [("a", "alpha"), ("b", "beta")] {
        std::fs::create_dir_all(s.wt.join(dir)).unwrap();
        base = s.commit_in_worktree(
            &format!("{dir}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\n"),
            name,
        );
    }
    seed_log(&s.root, NOW, "ss.s0", &[&format!("commit={base}")]);
    let sha = s.commit_in_worktree("a/x.rs", "x\n", "s1");
    seed_log(&s.root, NOW, "ss.s1", &[&format!("commit={sha}")]);
    seed_log(&s.root, NOW, "ss", &[&format!("pushed={sha}")]);
    for to in ["inprogress", "inreview", "done"] {
        s.ok(&["board", "move", "ss.s1", to]);
    }
    std::fs::write(s.wt.join("b/y.rs"), "y\n").unwrap();
    let out = s.run(&["ci", "gauntlet", "ss", "--level", "review"]).text();
    assert!(out.starts_with("gauntlet=pass"), "{out}");
    assert!(
        gauntlet_log(&s).contains("-p alpha -p beta"),
        "{}",
        gauntlet_log(&s)
    );
}

/// GA-R-102 — auto-continues in a row never raise a stop.
#[test]
fn it_stage_stop_never_counts_auto_continues() {
    let s = Scratch::new(5);
    s.remove_artifact("plan.summary.md");
    for n in 1..=4 {
        assert_eq!(s.say(&["stage", "stop", "ss", &format!("s{n}")]), "auto");
    }
}

#[test]
fn it_sha_check_and_reconcile() {
    let s = Scratch::new(2);
    let s1 = s.commit_in_worktree("f1.txt", "x\n", "s1");
    seed_log(&s.root, NOW, "ss.s1", &[&format!("commit={s1}")]);
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

    seed_log(&s.root, NOW, "ss.s0", &[&format!("commit={s1}")]);
    assert_eq!(
        s.run(&["run", "reconcile", "ss"]).text(),
        "reconcile=clean next=s1\nexit=0"
    );
    seed_log(&s.root, NOW, "ss.s2", &["commit=deadbee"]);
    seed_log(&s.root, NOW, "ss", &["gauntlet=pass", &format!("sha={m}")]);
    assert_eq!(
        s.run(&["run", "reconcile", "ss"]).text(),
        format!(
            "differ row=commit s2 deadbee does not exist\ndiffer row=verified {m} not on feat/ss\nreconcile=differ n=2 next=s1\nexit=1"
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
    assert_eq!(
        s.run(&["run", "reconcile", "nosuch"]).text(),
        "reconcile=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
}

#[test]
fn it_reconcile_follows_the_worktree_field() {
    let s = Scratch::new(1);
    let other = s.root.join("elsewhere/ss");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    git(
        &s.root,
        &[
            "worktree",
            "move",
            s.wt.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    seed_set(&s.root, "ss", &[&format!("worktree={}", other.display())]);
    // s0 is done without a commit, which reconcile reports; the worktree
    // at the field's path is not reported missing
    let got = s.run(&["run", "reconcile", "ss"]).text();
    assert!(!got.contains("row=worktree"), "{got}");
    assert!(got.contains("next=s1"), "{got}");
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
    seed_log(&s.root, NOW, "demo", &["ci=1"]);
    assert!(cleanup().starts_with("cleanup=refused reason=ci-red hint="));
    assert!(s.wt.is_dir());
    assert_eq!(s.say(&["run", "metrics"]), "");

    seed_log(&s.root, NOW, "demo", &["ci=0"]);
    assert_eq!(cleanup(), "cleanup=done archive=1 issue=12");
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
            .starts_with("export=refused reason=no-archive hint=")
    );
}

#[test]
fn it_cleanup_checks_the_worktree_field() {
    let s = pushed_run();
    let other = s.root.join("elsewhere/demo");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    git(
        &s.root,
        &[
            "worktree",
            "move",
            s.wt.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    seed_set(&s.root, "demo", &[&format!("worktree={}", other.display())]);
    std::fs::write(other.join("y.txt"), "y\n").unwrap();
    assert!(
        s.say(&["run", "cleanup", "demo"])
            .starts_with("cleanup=refused reason=uncommitted-changes hint=")
    );
    assert!(other.is_dir());
}

#[test]
fn it_cleanup_of_an_epic_needs_no_ci() {
    let s = pushed_run();
    assert_eq!(
        s.say(&["run", "cleanup", "lb"]),
        "cleanup=done archive=1 issue=70"
    );
    assert_eq!(
        s.say(&["run", "cleanup", "lb"]),
        "cleanup=done archive=none"
    );
}

/// Stdout of `ferrule <args>` with the fake `gh` on the path, the trailing
/// newline dropped.
fn say_on(s: &Scratch, gh: &FakeGh, args: &[&str]) -> String {
    let out = gh.on(s.run(args)).output();
    String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
}

/// Serves issue 12 as `state` (`OPEN` or `CLOSED`).
fn serve_issue(gh: &FakeGh, state: &str) {
    gh.serve(
        &["issue", "view", "12", "--json", GH_FIELDS],
        &gh_json(&format!(
            "#12 Title [{state}]\nhttps://github.com/o/r/issues/12\nby someone\n\nbody\n"
        )),
    );
}

/// RU-R-001, RU-R-012 — a filed request run archives with no CI result
/// and no host call, naming its issue.
#[test]
fn it_cleanup_of_a_filed_request_run_names_its_issue() {
    let s = pushed_run();
    let gh = FakeGh::new();
    seed_set(&s.root, "demo", &["pr=", "kind=request"]);
    assert_eq!(
        say_on(&s, &gh, &["run", "cleanup", "demo"]),
        "cleanup=done archive=1 issue=12"
    );
    assert!(s.say(&["run", "metrics"]).ends_with(" ci=-"));
    assert_eq!(gh.log(), "");
}

/// RU-R-001 — an unfiled request run archives without an issue token.
#[test]
fn it_cleanup_of_an_unfiled_request_run() {
    let s = pushed_run();
    let gh = FakeGh::new();
    seed_set(&s.root, "demo", &["pr=", "issue=", "kind=request"]);
    assert_eq!(
        say_on(&s, &gh, &["run", "cleanup", "demo"]),
        "cleanup=done archive=1"
    );
}

/// RU-R-002, RU-R-012 — a run without a PR or an issue needs no CI.
#[test]
fn it_cleanup_without_pr_or_issue_needs_no_ci() {
    let s = pushed_run();
    let gh = FakeGh::new();
    seed_set(&s.root, "demo", &["pr=", "issue="]);
    assert_eq!(
        say_on(&s, &gh, &["run", "cleanup", "demo"]),
        "cleanup=done archive=1"
    );
    assert!(s.say(&["run", "metrics"]).ends_with(" ci=-"));
}

/// RU-R-002 — a run without a PR whose issue is closed archives.
#[test]
fn it_cleanup_without_pr_and_closed_issue() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_issue(&gh, "CLOSED");
    seed_set(&s.root, "demo", &["pr="]);
    assert_eq!(
        say_on(&s, &gh, &["run", "cleanup", "demo"]),
        "cleanup=done archive=1 issue=12"
    );
}

/// RU-R-003 — a run without a PR whose issue is open is refused.
#[test]
fn it_cleanup_refuses_while_the_issue_is_open() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_issue(&gh, "OPEN");
    seed_set(&s.root, "demo", &["pr="]);
    let state = s.say(&["board", "field", "demo", "state"]);
    let got = say_on(&s, &gh, &["run", "cleanup", "demo"]);
    assert!(
        got.starts_with("cleanup=refused reason=issue-open issue=12 hint="),
        "{got}"
    );
    assert!(s.wt.is_dir());
    assert_eq!(s.say(&["board", "field", "demo", "state"]), state);
}

/// S1, S2, RU-R-061, RU-R-063, RU-R-065, RU-R-067 — a PR run with no `ci=`
/// line is refused with the waiver hint, then archives on a `ci-waived` line
/// and records `ci=-`.
#[test]
fn it_cleanup_without_ci_names_the_waiver_then_archives_on_it() {
    let s = pushed_run();
    let got = s.say(&["run", "cleanup", "demo"]);
    assert!(
        got.starts_with("cleanup=refused reason=no-ci hint="),
        "{got}"
    );
    assert!(
        got.contains(
            "hint=record CI first: ferrule ci wait demo; on the user's word only: \
             ferrule board log demo ci-waived -- <why>"
        ),
        "{got}"
    );
    assert!(s.wt.is_dir());
    s.ok(&["board", "log", "demo", "ci-waived", "--", "no checks"]);
    assert_eq!(
        s.say(&["run", "cleanup", "demo"]),
        "cleanup=done archive=1 issue=12"
    );
    let metrics = s.say(&["run", "metrics"]);
    assert!(metrics.ends_with(" ci=-"), "{metrics}");
}

/// S3, RU-R-062, RU-R-064, RU-E-033 — a `ci=` line after a waiver decides
/// again; a new `ci-waived` line after a non-zero one lets cleanup through.
#[test]
fn it_cleanup_ci_after_a_waiver_decides_again() {
    let s = pushed_run();
    s.ok(&["board", "log", "demo", "ci-waived", "--", "no checks"]);
    seed_log(&s.root, NOW, "demo", &["ci=1"]);
    let got = s.say(&["run", "cleanup", "demo"]);
    assert!(
        got.starts_with("cleanup=refused reason=ci-red hint="),
        "{got}"
    );
    seed_log(
        &s.root,
        NOW,
        "demo",
        &["ci-waived", "--", "red on main too"],
    );
    assert_eq!(
        s.say(&["run", "cleanup", "demo"]),
        "cleanup=done archive=1 issue=12"
    );
    let metrics = s.say(&["run", "metrics"]);
    assert!(metrics.ends_with(" ci=waived"), "{metrics}");
}

/// RU-R-004, RU-R-005 — an unreadable issue refuses until the user's
/// `issue-waived` line, then the run archives without a second host call.
#[test]
fn it_cleanup_with_unreadable_issue_needs_the_users_waiver() {
    let s = pushed_run();
    let gh = FakeGh::new();
    gh.serve_err(
        &["issue", "view", "12", "--json", GH_FIELDS],
        "HTTP 401: Bad credentials",
        1,
    );
    seed_set(&s.root, "demo", &["pr="]);
    let got = say_on(&s, &gh, &["run", "cleanup", "demo"]);
    assert!(
        got.starts_with("cleanup=refused reason=issue-unreadable issue=12 detail="),
        "{got}"
    );
    assert!(
        got.contains("hint=on the user's word only: ferrule board log demo issue-waived -- <why>"),
        "{got}"
    );
    s.ok(&[
        "board",
        "log",
        "demo",
        "issue-waived",
        "--",
        "user approved",
    ]);
    assert_eq!(
        say_on(&s, &gh, &["run", "cleanup", "demo"]),
        "cleanup=done archive=1 issue=12"
    );
    assert_eq!(gh.log().matches("issue view").count(), 1, "{}", gh.log());
}

/// RU-E-001 — a run without a PR still gets the unpushed-work refusals.
#[test]
fn it_cleanup_without_a_pr_still_refuses_unpushed_work() {
    let s = pushed_run();
    let gh = FakeGh::new();
    seed_set(&s.root, "demo", &["pr=", "issue="]);
    std::fs::write(s.wt.join("y.txt"), "y\n").unwrap();
    assert!(
        say_on(&s, &gh, &["run", "cleanup", "demo"])
            .starts_with("cleanup=refused reason=uncommitted-changes hint=")
    );
    git(&s.wt, &["add", "y.txt"]);
    git(&s.wt, &["commit", "-q", "-m", "more"]);
    assert!(
        say_on(&s, &gh, &["run", "cleanup", "demo"])
            .starts_with("cleanup=refused reason=unpushed-commits hint=")
    );
    assert!(
        s.say(&["run", "export", "demo"])
            .starts_with("export=refused reason=no-archive hint=")
    );
}

/// The latest archive id of `demo`.
fn latest_archive(s: &Scratch) -> usize {
    let to = s.root.join("o2");
    let out = s.say(&["run", "export", "demo", "--to", to.to_str().unwrap()]);
    out.trim_start_matches("export=done archive=")
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap_or_else(|_| panic!("{out}"))
}

#[test]
fn it_cleanup_resumes_after_the_branch_cannot_be_deleted() {
    let s = pushed_run();
    seed_log(&s.root, NOW, "demo", &["ci=0"]);
    let lock = s.root.join(".git/refs/heads/feat/demo.lock");
    std::fs::write(&lock, "").unwrap();
    let first = s.say(&["run", "cleanup", "demo"]);
    assert!(
        first.starts_with("cleanup=refused reason=branch-delete-failed detail="),
        "{first}"
    );
    assert!(!s.wt.exists());
    assert!(has_ref(&s.root, "refs/heads/feat/demo"));
    assert!(s.root.join(".ferrule/runs/demo").exists());
    std::fs::remove_file(&lock).unwrap();
    assert_eq!(
        s.say(&["run", "cleanup", "demo"]),
        "cleanup=done archive=1 resumed"
    );
    assert!(!has_ref(&s.root, "refs/heads/feat/demo"));
    assert!(!s.root.join(".ferrule/runs/demo").exists());
    assert_eq!(
        s.say(&["run", "cleanup", "demo"]),
        "cleanup=done archive=none"
    );
    assert_eq!(latest_archive(&s), 1);
}

#[test]
fn it_cleanup_resumes_after_the_worktree_cannot_be_removed() {
    let s = pushed_run();
    seed_log(&s.root, NOW, "demo", &["ci=0"]);
    git(&s.root, &["worktree", "lock", s.wt.to_str().unwrap()]);
    let first = s.say(&["run", "cleanup", "demo"]);
    assert!(
        first.starts_with("cleanup=refused reason=worktree-remove-failed detail="),
        "{first}"
    );
    assert!(s.wt.is_dir());
    assert_eq!(s.say(&["board", "field", "demo", "state"]), "");
    git(&s.root, &["worktree", "unlock", s.wt.to_str().unwrap()]);
    assert_eq!(
        s.say(&["run", "cleanup", "demo"]),
        "cleanup=done archive=1 resumed"
    );
    assert!(!s.wt.exists());
    assert!(!has_ref(&s.root, "refs/heads/feat/demo"));
    assert_eq!(latest_archive(&s), 1);
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
    assert!(g(&[]).starts_with("gauntlet=refused reason=no-steps hint="));
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
        format!("{pass} dirty\nexit=0"),
        "an untracked file is dirty"
    );
    assert_eq!(
        g(&[]),
        format!("{pass} dirty\nexit=0"),
        "a dirty pass is never cached"
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
    assert!(g(&[]).starts_with("gauntlet=refused reason=no-worktree hint="));
}

/// Three labeled steps, each printing its own name, and one with no label.
const LEVELS: &str = r#"
[[gauntlet.step]]
name = "pushed"
cmd = ["sh", "-c", "echo pushed-out"]
at = ["push"]

[[gauntlet.step]]
name = "reviewed"
cmd = ["sh", "-c", "echo reviewed-out"]
at = ["review"]

[[gauntlet.step]]
name = "everywhere"
cmd = ["sh", "-c", "echo everywhere-out"]
at = ["push", "review"]

[[gauntlet.step]]
name = "plain"
cmd = ["sh", "-c", "echo plain-out"]
"#;

fn gauntlet_log(s: &Scratch) -> String {
    std::fs::read_to_string(s.root.join(".ferrule/runs/ss/gauntlet.log")).unwrap_or_default()
}

/// A worktree holding a crate whose one test is not yet committed.
fn crate_scratch(stack: &str) -> Scratch {
    let s = Scratch::new(1);
    s.config(&format!("{stack}{LEVELS}"));
    std::fs::write(
        s.wt.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir(s.wt.join("src")).unwrap();
    std::fs::write(
        s.wt.join("src/lib.rs"),
        "#[test]\nfn t_added() {\n    assert!(true);\n}\n",
    )
    .unwrap();
    s
}

/// GA-R-082 — `--level push --stage s1` runs the push steps and the test
/// the stage's diff adds, and no other step.
#[test]
fn it_gauntlet_stage_checks_run_push_steps_and_new_tests() {
    let s = crate_scratch("stack = \"rust\"\n");
    let out = s
        .run(&["ci", "gauntlet", "ss", "--level", "push", "--stage", "s1"])
        .text();
    assert!(out.starts_with("gauntlet=pass cov=? sha="), "{out}");
    let log = gauntlet_log(&s);
    assert!(log.contains("### pushed:"), "{log}");
    assert!(log.contains("### everywhere:"), "{log}");
    assert!(
        log.contains("### test:t_added: cargo test --workspace -- t_added"),
        "{log}"
    );
    assert!(
        !log.contains("### reviewed:") && !log.contains("### plain:"),
        "{log}"
    );
}

/// GA-R-083 — `--level review` runs the steps labeled review and no stage test.
#[test]
fn it_gauntlet_review_level_runs_only_review_steps() {
    let s = crate_scratch("stack = \"rust\"\n");
    let out = s.run(&["ci", "gauntlet", "ss", "--level", "review"]).text();
    assert!(out.starts_with("gauntlet=pass cov=? sha="), "{out}");
    let log = gauntlet_log(&s);
    assert!(
        log.contains("### reviewed:") && log.contains("### everywhere:"),
        "{log}"
    );
    assert!(
        !log.contains("### pushed:") && !log.contains("### plain:"),
        "{log}"
    );
    assert!(!log.contains("### test:"), "{log}");
}

/// GA-R-084 — the full level, the default, runs every step, labeled or not.
#[test]
fn it_gauntlet_full_level_runs_every_step() {
    let s = crate_scratch("stack = \"rust\"\n");
    let out = s.run(&["ci", "gauntlet", "ss"]).text();
    assert!(out.starts_with("gauntlet=pass cov=? sha="), "{out}");
    let log = gauntlet_log(&s);
    for name in ["pushed", "reviewed", "everywhere", "plain"] {
        assert!(log.contains(&format!("### {name}:")), "{name}: {log}");
    }
    assert!(!log.contains("### test:"), "{log}");
}

/// GA-R-090 — a pass or failure at full is logged on a clean worktree only,
/// never at another level.
#[test]
fn it_gauntlet_full_logs_on_a_clean_tree_only() {
    let s = Scratch::new(1);
    s.config("[[gauntlet.step]]\nname = \"ok\"\ncmd = [\"true\"]\nat = [\"push\"]\n");
    let logged = |s: &Scratch| s.say(&["board", "show", "ss"]).matches("gauntlet=").count();
    assert!(
        s.run(&["ci", "gauntlet", "ss", "--level", "push"])
            .text()
            .ends_with("exit=0")
    );
    assert_eq!(logged(&s), 0, "push logs nothing");
    assert!(s.run(&["ci", "gauntlet", "ss"]).text().ends_with("exit=0"));
    assert_eq!(logged(&s), 1);
    assert!(
        s.say(&["board", "show", "ss"])
            .contains("gauntlet=pass cov=? sha=")
    );
    std::fs::write(s.wt.join("b.txt"), "b\n").unwrap();
    assert!(
        s.run(&["ci", "gauntlet", "ss", "--force"])
            .text()
            .ends_with("exit=0")
    );
    assert_eq!(logged(&s), 1, "a dirty pass is not logged");
    std::fs::remove_file(s.wt.join("b.txt")).unwrap();
    s.config("[[gauntlet.step]]\nname = \"bad\"\ncmd = [\"false\"]\n");
    assert!(s.run(&["ci", "gauntlet", "ss"]).text().ends_with("exit=1"));
    assert_eq!(logged(&s), 2);
    assert!(s.say(&["board", "show", "ss"]).contains("step=bad"));
}

/// GA-E-009 — steps naming no level run only at full: push and review pass
/// without running or logging anything.
#[test]
fn it_gauntlet_unlabeled_steps_run_only_at_full() {
    let s = Scratch::new(1);
    s.config("[[gauntlet.step]]\nname = \"plain\"\ncmd = [\"true\"]\n");
    for level in ["push", "review"] {
        let out = s.run(&["ci", "gauntlet", "ss", "--level", level]).text();
        assert!(out.starts_with("gauntlet=pass cov=? sha="), "{out}");
        assert_eq!(gauntlet_log(&s), "", "{level} ran a step");
    }
    assert!(s.run(&["ci", "gauntlet", "ss"]).text().ends_with("exit=0"));
    assert!(gauntlet_log(&s).contains("### plain:"));
}

/// GA-E-010 — off a Rust stack a stage's checks run only the push steps.
#[test]
fn it_gauntlet_stage_tests_skipped_off_rust() {
    let s = crate_scratch("");
    let out = s
        .run(&["ci", "gauntlet", "ss", "--level", "push", "--stage", "s1"])
        .text();
    assert!(out.starts_with("gauntlet=pass cov=? sha="), "{out}");
    let log = gauntlet_log(&s);
    assert!(log.contains("### pushed:"), "{log}");
    assert!(!log.contains("### test:"), "{log}");
}

#[test]
fn it_gauntlet_cache_follows_the_steps_and_forgets_on_failure() {
    let s = Scratch::new(1);
    let g = |extra: &[&str]| {
        let mut args = vec!["ci", "gauntlet", "ss"];
        args.extend_from_slice(extra);
        s.run(&args).text()
    };
    let flag = s.root.join("fail.flag");
    let steps = format!(
        "{STEPS}\n[[gauntlet.step]]\nname = \"flag\"\ncmd = [\"sh\", \"-c\", \"test ! -f {}\"]\n",
        flag.display()
    );
    s.config(&steps);
    let sha = git(&s.wt, &["rev-parse", "--short", "HEAD"]);
    let pass = format!("gauntlet=pass cov=81.20% sha={sha}");
    assert_eq!(g(&[]), format!("{pass}\nexit=0"));
    assert_eq!(g(&[]), format!("{pass} cached\nexit=0"));

    s.config(&format!(
        "{steps}\n[[gauntlet.step]]\nname = \"x\"\ncmd = [\"true\"]\n"
    ));
    assert_eq!(
        g(&[]),
        format!("{pass}\nexit=0"),
        "new steps miss the cache"
    );
    assert_eq!(g(&[]), format!("{pass} cached\nexit=0"));

    std::fs::write(&flag, "").unwrap();
    assert!(g(&["--force"]).starts_with(&format!("gauntlet=fail step=flag sha={sha}")));
    std::fs::remove_file(&flag).unwrap();
    assert_eq!(
        g(&[]),
        format!("{pass}\nexit=0"),
        "a failure forgets the cached pass"
    );
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
fn it_run_diff_uncommitted_reads_the_specs_a_base_diff_omits() {
    let s = Scratch::new(1);
    std::fs::create_dir_all(s.wt.join("docs/specs")).unwrap();
    s.commit_in_worktree("docs/specs/a.md", "**MB-R-002** — Polls.\n", "spec");
    s.commit_in_worktree("f1.txt", "one\n", "add f1");
    std::fs::write(
        s.wt.join("docs/specs/a.md"),
        "**MB-R-002** — Polls twice.\n",
    )
    .unwrap();
    std::fs::write(s.wt.join("docs/specs/b.md"), "**MB-R-003** — New.\n").unwrap();
    std::fs::write(s.wt.join("f1.txt"), "changed\n").unwrap();
    let base = s.say(&["run", "diff", "ss", "--specs"]);
    assert!(!base.contains("Polls twice"), "{base}");
    let wip = s.say(&["run", "diff", "ss", "--uncommitted", "--specs"]);
    assert!(wip.contains("+**MB-R-002** — Polls twice."), "{wip}");
    assert!(wip.contains("?? docs/specs/b.md\n"), "{wip}");
    assert!(!wip.contains("MB-R-003"), "{wip}");
    assert!(!wip.contains("+changed"), "{wip}");
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
        "events=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
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
    seed_log(&s.root, NOW, "ss", &["pr=draft"]);
    seed_log(&s.root, NOW, "ss", &["gauntlet=pass", "cov=80%", "sha=abc"]);
    seed_log(&s.root, NOW, "ss", &["ci=0"]);
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

/// An unreadable ferrule.toml or state database is an internal error:
/// exit 4 with `ferrule: <error>` on stderr, never a refusal.
#[test]
fn it_internal_errors_exit_4() {
    let s = Scratch::new(1);
    s.config("main_branch = [");
    let out = s.run(&["run", "status", "ss"]).output();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(4), "{err}");
    assert!(err.starts_with("ferrule: ferrule.toml: "), "{err}");
    assert!(out.stdout.is_empty());

    std::fs::remove_file(s.root.join(".ferrule/ferrule.toml")).unwrap();
    std::fs::write(s.root.join(".ferrule/state.db"), "not a database").unwrap();
    let out = s.run(&["board", "list"]).output();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(4), "{err}");
    assert!(err.starts_with("ferrule: state database: "), "{err}");
}

#[test]
fn it_refuses_a_slug_that_is_no_path_segment() {
    let s = Scratch::new(1);
    let outside = s.root.join("sentinel");
    std::fs::create_dir(&outside).unwrap();
    for slug in ["..", "../../evil", "a/b", ".", "fix.s2", "-x", "a b", ""] {
        let card = format!("{slug}.s1");
        for args in [
            vec!["run", "init", "--", slug],
            vec!["run", "cleanup", "--", slug],
            vec!["run", "status", "--", slug],
            vec!["board", "new", "--", &card],
        ] {
            let out = s.run(&args).output();
            assert_eq!(out.status.code(), Some(2), "{args:?}");
        }
    }
    assert!(outside.is_dir());
    assert!(s.root.join(".ferrule/state.db").is_file());
    assert!(!s.root.join("evil").exists());
}

#[test]
fn it_cleanup_archives_no_symlink() {
    let s = pushed_run();
    let run_dir = s.root.join(".ferrule/runs/demo");
    std::fs::create_dir_all(&run_dir).unwrap();
    let secret = s.root.join("secret.txt");
    std::fs::write(&secret, "secret\n").unwrap();
    std::os::unix::fs::symlink(&secret, run_dir.join("link.md")).unwrap();
    std::os::unix::fs::symlink(&s.root, run_dir.join("loop")).unwrap();
    seed_log(&s.root, NOW, "demo", &["ci=0"]);
    assert_eq!(
        s.say(&["run", "cleanup", "demo"]),
        "cleanup=done archive=1 issue=12"
    );
    let to = s.root.join("out");
    s.say(&["run", "export", "demo", "--to", to.to_str().unwrap()]);
    assert!(!to.join("link.md").exists());
    assert!(secret.is_file());
    assert!(s.root.join(".git").is_dir());
}

/// SG-E-013 — a cleaned-up run's open live check allows nothing.
#[test]
fn it_archived_run_opens_no_live_check() {
    use std::io::Write;
    use std::process::Stdio;
    let s = pushed_run();
    seed_set(&s.root, "demo", &["live-repo=o/r"]);
    seed_log(&s.root, NOW, "demo", &["ci=0"]);
    assert_eq!(
        s.say(&["run", "live-check", "demo", "start"]),
        "live-check=open repo=o/r"
    );
    let hook = || {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "gh api -X POST repos/o/r/issues", "description": "d"},
            "cwd": s.root,
        });
        let mut child = Command::new(env!("CARGO_BIN_EXE_ferrule"))
            .args(["hook", "shell"])
            .current_dir(&s.root)
            .env("CLAUDE_PROJECT_DIR", &s.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(hook().is_empty(), "an open check allows the write");
    assert!(
        s.say(&["run", "cleanup", "demo"])
            .starts_with("cleanup=done archive=1")
    );
    assert!(hook().contains("deny"), "an archived run allows nothing");
}

const CLOSE: &str = ferrule_core::run::abandon::CLOSE_COMMENT;

/// Serves PR 34 as `state` (`OPEN`, `MERGED` or `CLOSED`).
fn serve_pr(gh: &FakeGh, state: &str) {
    gh.serve(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        &format!("{{\"state\":\"{state}\",\"isDraft\":false,\"body\":\"\"}}"),
    );
}

const CLOSE_ARGS: fn() -> [&'static str; 5] = || ["pr", "close", "34", "--comment", CLOSE];

/// Serves the close of PR 34.
fn serve_close(gh: &FakeGh) {
    gh.serve(&CLOSE_ARGS(), "");
}

fn remote_has(s: &Scratch, branch: &str) -> bool {
    !git(
        &s.root,
        &["ls-remote", "origin", &format!("refs/heads/{branch}")],
    )
    .is_empty()
}

fn state_of(s: &Scratch, id: &str) -> String {
    s.say(&["board", "field", id, "state"])
}

/// Nothing of run `demo` changed: card, worktree, local and remote branch.
fn assert_demo_intact(s: &Scratch) {
    assert_eq!(state_of(s, "demo"), "inprogress");
    assert!(s.wt.is_dir());
    assert!(has_ref(&s.root, "refs/heads/feat/demo"));
    assert!(remote_has(s, "feat/demo"));
}

/// S13, RU-R-045, RU-R-046 — without `--confirm` the command lists the unsaved
/// work and every effect, in order, and changes nothing.
#[test]
fn it_abandon_without_confirm_lists_and_changes_nothing() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    let sha = s.commit_in_worktree("more.txt", "m\n", "more");
    std::fs::write(s.wt.join("y.txt"), "y\n").unwrap();
    let got = say_on(&s, &gh, &["run", "abandon", "demo"]);
    let want = format!(
        "abandon=plan\nuncommitted y.txt\nunpushed {sha} more\nclose-pr 34\ndelete-remote-branch feat/demo\ndelete-branch feat/demo\nremove-worktree {}\nhint=on the user's word only: ferrule run abandon demo --confirm",
        s.wt.display()
    );
    assert_eq!(got, want);
    assert_demo_intact(&s);
    assert!(s.wt.join("y.txt").is_file());
    assert!(!gh.log().contains("pr close"), "{}", gh.log());
    assert_eq!(s.say(&["board", "field", "demo", "pr"]), "34");
}

/// Lines of an abandon plan that name uncommitted files.
fn uncommitted_lines(s: &Scratch, gh: &FakeGh) -> Vec<String> {
    say_on(s, gh, &["run", "abandon", "demo"])
        .lines()
        .filter(|l| l.starts_with("uncommitted "))
        .map(str::to_owned)
        .collect()
}

/// S1, RU-R-060 — a staged rename is one `uncommitted` line naming the new path.
#[test]
fn it_abandon_plan_lists_a_staged_rename_by_its_new_path() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    s.commit_in_worktree("a.txt", "a\n", "add a");
    git(&s.wt, &["mv", "a.txt", "b.txt"]);
    assert_eq!(uncommitted_lines(&s, &gh), ["uncommitted b.txt"]);
}

/// S2, RU-R-059 — a path with a space or non-ASCII letter is listed plain.
#[test]
fn it_abandon_plan_lists_special_paths_plain() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    std::fs::write(s.wt.join("s p.txt"), "x\n").unwrap();
    std::fs::write(s.wt.join("ü.txt"), "x\n").unwrap();
    assert_eq!(
        uncommitted_lines(&s, &gh),
        ["uncommitted s p.txt", "uncommitted ü.txt"]
    );
}

/// S3, RU-E-030 — a path with a line break is listed quoted on one line.
#[cfg(unix)]
#[test]
fn it_abandon_plan_quotes_a_path_with_a_line_break() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    std::fs::write(s.wt.join("n\nl.txt"), "x\n").unwrap();
    let out = say_on(&s, &gh, &["run", "abandon", "demo"]);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.contains(&"uncommitted \"n\\nl.txt\""), "{out}");
    assert_eq!(
        lines.len(),
        1 + 1 + 4 + 1 + usize::from(out.contains("unpushed")),
        "{out}"
    );
}

/// RU-E-031 — an untracked directory is one `uncommitted <dir>/` line.
#[test]
fn it_abandon_plan_lists_an_untracked_dir_once() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    std::fs::create_dir(s.wt.join("d")).unwrap();
    std::fs::write(s.wt.join("d/x.txt"), "x\n").unwrap();
    std::fs::write(s.wt.join("d/y.txt"), "y\n").unwrap();
    assert_eq!(uncommitted_lines(&s, &gh), ["uncommitted d/"]);
}

/// RU-E-032 — a move the index does not hold is two lines, old then new.
#[test]
fn it_abandon_plan_lists_an_unstaged_move_as_two_lines() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    s.commit_in_worktree("a.txt", "a\n", "add a");
    std::fs::rename(s.wt.join("a.txt"), s.wt.join("b.txt")).unwrap();
    assert_eq!(
        uncommitted_lines(&s, &gh),
        ["uncommitted a.txt", "uncommitted b.txt"]
    );
}

/// S14, RU-R-047, RU-R-049, RU-R-050, RU-R-051, RU-R-052, RU-R-053, RU-R-055,
/// RU-E-022 — a confirmed abandon of a run whose worktree is gone archives it
/// as abandoned, deletes both branches, comments on and closes the PR, and
/// leaves the open issue alone.
#[test]
fn it_abandon_confirm_clears_a_run_whose_worktree_is_gone() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_issue(&gh, "OPEN");
    serve_pr(&gh, "OPEN");
    serve_close(&gh);
    git(
        &s.root,
        &["worktree", "remove", "--force", s.wt.to_str().unwrap()],
    );
    assert_eq!(
        say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]),
        "abandon=done archive=1 issue=12"
    );
    assert!(!has_ref(&s.root, "refs/heads/feat/demo"));
    assert!(!remote_has(&s, "feat/demo"));
    let log = gh.log();
    assert!(log.contains("pr close 34 --comment"), "{log}");
    assert!(!log.lines().any(|l| l.starts_with("issue")), "{log}");
    assert_eq!(state_of(&s, "demo"), "");
    let metrics = s.say(&["run", "metrics"]);
    assert!(metrics.ends_with(" ci=-"), "{metrics}");
    let to = s.root.join("out");
    s.say(&["run", "export", "demo", "--to", to.to_str().unwrap()]);
    let card = std::fs::read_to_string(to.join("cards/demo.md")).unwrap();
    assert!(card.lines().any(|l| l.ends_with(" abandoned")), "{card}");
}

/// RU-R-048 — a confirmed abandon removes a worktree holding uncommitted and
/// unpushed work.
#[test]
fn it_abandon_confirm_removes_a_dirty_worktree() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    serve_close(&gh);
    s.commit_in_worktree("more.txt", "m\n", "more");
    std::fs::write(s.wt.join("y.txt"), "y\n").unwrap();
    let got = say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]);
    assert!(got.starts_with("abandon=done archive=1"), "{got}");
    assert!(!s.wt.exists());
}

/// RU-E-020 — a run without a PR or a remote branch skips both steps and
/// lists nothing for them.
#[test]
fn it_abandon_skips_a_missing_pr_and_remote_branch() {
    let s = pushed_run();
    let gh = FakeGh::new();
    seed_set(&s.root, "demo", &["pr="]);
    git(&s.root, &["push", "-q", "origin", "--delete", "feat/demo"]);
    let plan = say_on(&s, &gh, &["run", "abandon", "demo"]);
    assert!(!plan.contains("close-pr"), "{plan}");
    assert!(!plan.contains("delete-remote-branch"), "{plan}");
    assert!(plan.contains("delete-branch feat/demo"), "{plan}");
    let got = say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]);
    assert!(got.starts_with("abandon=done archive=1"), "{got}");
    assert_eq!(gh.log(), "");
}

/// RU-E-023 — a merged PR refuses the abandon in both modes, changing nothing.
#[test]
fn it_abandon_refuses_a_merged_pr() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "MERGED");
    for args in [
        &["run", "abandon", "demo"][..],
        &["run", "abandon", "demo", "--confirm"],
    ] {
        let got = say_on(&s, &gh, args);
        assert!(
            got.starts_with("abandon=refused reason=merged pr=34 hint="),
            "{got}"
        );
        assert!(got.contains("ferrule run cleanup demo"), "{got}");
        assert_demo_intact(&s);
    }
}

/// RU-E-024, RU-E-025 — an epic's parent is refused in both modes; a slice
/// run is abandoned as any other.
#[test]
fn it_abandon_refuses_an_epic_parent_and_takes_a_slice_run() {
    let s = pushed_run();
    let gh = FakeGh::new();
    for args in [
        &["run", "abandon", "lb"][..],
        &["run", "abandon", "lb", "--confirm"],
    ] {
        let got = say_on(&s, &gh, args);
        assert!(
            got.starts_with("abandon=refused reason=epic hint="),
            "{got}"
        );
    }
    assert_ne!(state_of(&s, "lb"), "");
    assert_demo_intact(&s);
    assert_eq!(gh.log(), "");
    serve_pr(&gh, "OPEN");
    serve_close(&gh);
    seed_set(&s.root, "demo", &["kind=slice"]);
    let got = say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]);
    assert!(got.starts_with("abandon=done archive="), "{got}");
}

/// RU-E-027 — a PR whose state cannot be read refuses the abandon.
#[test]
fn it_abandon_refuses_an_unreadable_pr() {
    let s = pushed_run();
    let gh = FakeGh::new();
    gh.serve_err(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        "boom",
        1,
    );
    for args in [
        &["run", "abandon", "demo"][..],
        &["run", "abandon", "demo", "--confirm"],
    ] {
        let got = say_on(&s, &gh, args);
        assert!(
            got.starts_with("abandon=refused reason=pr-unreadable detail="),
            "{got}"
        );
        assert_demo_intact(&s);
    }
}

/// RU-E-028 — a remote branch that cannot be looked up refuses the abandon.
#[test]
fn it_abandon_refuses_an_unreachable_remote() {
    let s = pushed_run();
    let gh = FakeGh::new();
    seed_set(&s.root, "demo", &["pr="]);
    let missing = s.root.join("no-such-remote.git");
    git(
        &s.root,
        &["remote", "set-url", "origin", missing.to_str().unwrap()],
    );
    for args in [
        &["run", "abandon", "demo"][..],
        &["run", "abandon", "demo", "--confirm"],
    ] {
        let got = say_on(&s, &gh, args);
        assert!(
            got.starts_with("abandon=refused reason=remote-unreadable branch=feat/demo hint="),
            "{got}"
        );
        assert_eq!(state_of(&s, "demo"), "inprogress");
        assert!(s.wt.is_dir());
        assert!(has_ref(&s.root, "refs/heads/feat/demo"));
    }
}

/// RU-E-021 — a rerun after a failed step performs only the steps still
/// missing.
#[test]
fn it_abandon_rerun_finishes_only_the_missing_steps() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_issue(&gh, "OPEN");
    serve_pr(&gh, "OPEN");
    gh.serve_err(&CLOSE_ARGS(), "boom", 1);
    let first = say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]);
    assert!(
        first.starts_with("abandon=refused reason=close-pr-failed detail="),
        "{first}"
    );
    assert_demo_intact(&s);

    serve_pr(&gh, "CLOSED");
    git(&s.root, &["worktree", "lock", s.wt.to_str().unwrap()]);
    let second = say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]);
    assert!(
        second.starts_with("abandon=refused reason=worktree-remove-failed detail="),
        "{second}"
    );
    assert_eq!(state_of(&s, "demo"), "");
    assert!(!remote_has(&s, "feat/demo"));
    assert!(s.wt.is_dir());

    git(&s.root, &["worktree", "unlock", s.wt.to_str().unwrap()]);
    assert_eq!(
        say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]),
        "abandon=done archive=1 issue=12 resumed"
    );
    assert!(!s.wt.exists());
    assert!(!has_ref(&s.root, "refs/heads/feat/demo"));
    let closes = gh
        .log()
        .lines()
        .filter(|l| l.starts_with("pr close"))
        .count();
    assert_eq!(closes, 1, "{}", gh.log());
    assert_eq!(latest_archive(&s), 1);
}

/// RU-E-026 — a run already fully abandoned is reported so, in both modes,
/// with exit 0.
#[test]
fn it_abandon_of_an_abandoned_run_reports_already() {
    let s = pushed_run();
    let gh = FakeGh::new();
    serve_pr(&gh, "OPEN");
    serve_close(&gh);
    let done = say_on(&s, &gh, &["run", "abandon", "demo", "--confirm"]);
    assert!(done.starts_with("abandon=done archive=1"), "{done}");
    for args in [
        &["run", "abandon", "demo"][..],
        &["run", "abandon", "demo", "--confirm"],
    ] {
        let got = gh.on(s.run(args)).text();
        assert_eq!(got, "abandon=already archive=1\nexit=0");
    }
    assert_eq!(latest_archive(&s), 1);
}

/// RU-E-029 — a run with no card and no abandoned archive is refused.
#[test]
fn it_abandon_refuses_an_unknown_or_cleaned_up_run() {
    let s = pushed_run();
    let gh = FakeGh::new();
    let got = say_on(&s, &gh, &["run", "abandon", "nosuch"]);
    assert!(
        got.starts_with("abandon=refused reason=card hint="),
        "{got}"
    );
    seed_log(&s.root, NOW, "demo", &["ci=0"]);
    assert_eq!(
        s.say(&["run", "cleanup", "demo"]),
        "cleanup=done archive=1 issue=12"
    );
    for args in [
        &["run", "abandon", "demo"][..],
        &["run", "abandon", "demo", "--confirm"],
    ] {
        let got = say_on(&s, &gh, args);
        assert!(
            got.starts_with("abandon=refused reason=card hint="),
            "{got}"
        );
    }
}

/// RU-R-050 — on Bitbucket the PR gets its comment before it is declined.
#[test]
fn it_abandon_on_bitbucket_comments_then_declines() {
    let s = pushed_run();
    std::fs::write(
        s.root.join(".ferrule/ferrule.toml"),
        "[backend]\nhost = \"bitbucket\"\n",
    )
    .unwrap();
    let curl_dir = tempfile::tempdir().unwrap();
    let curl = curl_dir.path().join("curl");
    std::fs::write(
        &curl,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$CURL_DIR/log"
cat > /dev/null
case " $* " in
  *" -d "*|*" -X "*) echo '{}' ;;
  *) echo '{"state":"OPEN","draft":false,"description":""}' ;;
esac
"#,
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        curl_dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let got = s
        .run(&["run", "abandon", "demo", "--confirm"])
        .env("PATH", path)
        .env("CURL_DIR", curl_dir.path())
        .env(
            "FERRULE_BITBUCKET_CREDENTIALS",
            r#"{"workspace":"w","repoSlug":"r","username":"u","appPassword":"p"}"#,
        )
        .output();
    let got = String::from_utf8_lossy(&got.stdout).trim_end().to_owned();
    assert!(got.starts_with("abandon=done archive=1"), "{got}");
    let log = std::fs::read_to_string(curl_dir.path().join("log")).unwrap();
    let at = |needle: &str| log.lines().position(|l| l.contains(needle));
    let comment = at("pullrequests/34/comments").expect(&log);
    let decline = at("pullrequests/34/decline").expect(&log);
    assert!(comment < decline, "{log}");
}
