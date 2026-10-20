//! `gate next` replays the workflow's next-step scenarios; the expected
//! transcripts are the shell tests' with each script call replaced by the
//! ferrule command that took its place.

mod golden;

use std::fmt::Write as _;
use std::path::Path;

use golden::{Run, check, seed_log};

const NOW: &str = "2026-10-05T09:00";
const R: &str = ".claude/tasks/artifacts/ns";

/// One scratch board for run `ns` and the transcript the scenario writes.
struct Board {
    dir: tempfile::TempDir,
    transcript: String,
}

impl Board {
    fn new() -> Self {
        Board {
            dir: tempfile::tempdir().unwrap(),
            transcript: String::new(),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn ferrule(&self, args: &[&str]) -> Run {
        Run::new(args)
            .env("CLAUDE_PROJECT_DIR", self.root())
            .env("FERRULE_NOW", NOW)
            .cwd(self.root())
    }

    fn ok(&self, args: &[&str]) {
        assert_eq!(self.ferrule(args).text(), "exit=0", "{args:?}");
    }

    /// A fresh `ns`: parent in inprogress with `lane` (`-` for none) and
    /// `arch`, a plan summary with s0 and `rows` more stages, and a verdict.
    fn reset(&mut self, lane: &str, arch: &str, rows: u32, verdict: &str) {
        let transcript = std::mem::take(&mut self.transcript);
        *self = Board::new();
        self.transcript = transcript;
        let arch = format!("arch={arch}");
        let lane = format!("lane={lane}");
        let mut args = vec![
            "board",
            "new",
            "ns",
            "--state",
            "inprogress",
            "branch=feat/ns",
            "gate1=2026-10-01",
            &arch,
        ];
        if lane != "lane=-" {
            args.push(&lane);
        }
        self.ok(&args);
        let mut summary = format!(
            "# ns — plan summary (rev 1)\n\nDecide: {rows} stages.\n\n| Stage | What | Pins | Files | Stop |\n|---|---|---|---|---|\n"
        );
        for i in 0..=rows {
            let _ = writeln!(summary, "| s{i} | stage {i} | — | 1 (x) | — |");
        }
        self.artifact("plan.summary.md", &summary);
        self.verdict(verdict);
    }

    fn artifact(&self, name: &str, text: &str) {
        let dir = self.root().join(".ferrule/runs/ns");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn verdict(&self, kind: &str) {
        let text = if kind == "clean" {
            "Verdict: clean — ready for approval\n\nSpec — 2 of 2 pinned\nNot checked — nothing\n"
        } else {
            "Verdict: findings — 1 needs user\n\nNeeds user\n- 1. keep the alias?\n  - a) keep (recommended)\n  - b) drop\n"
        };
        self.artifact("review.verdict.md", text);
    }

    fn log(&self, args: &[&str]) {
        seed_log(self.root(), NOW, "ns", args);
    }

    /// `gate next` with the status words, recorded as the shell tests did.
    fn next(&mut self, args: &[&str]) {
        let _ = writeln!(self.transcript, "$ gate next {}", args.join(" "));
        let mut all = vec!["gate", "next"];
        all.extend_from_slice(args);
        let out = self.ferrule(&all).text();
        self.transcript.push_str(&out);
        self.transcript.push('\n');
    }

    /// The parent card's log lines.
    fn log_dump(&mut self) {
        self.transcript.push_str("log:\n");
        let shown = self.ferrule(&["board", "show", "ns"]).output();
        let shown = String::from_utf8_lossy(&shown.stdout).into_owned();
        for line in shown.split("---\n").nth(2).unwrap_or("").lines() {
            self.transcript.push_str(line);
            self.transcript.push('\n');
        }
    }

    fn finish(self, name: &str) {
        check(name, &format!("{}exit=0", self.transcript));
    }
}

fn file(name: &str) -> String {
    format!("file={R}/{name}")
}

fn summary(name: &str) -> String {
    format!("summary={R}/{name}")
}

#[test]
fn it_next_plan_rounds() {
    let mut b = Board::new();
    b.reset("full", "2026-10-02", 2, "clean");
    let (plan, ps, review, verdict) = (
        file("plan.md"),
        summary("plan.summary.md"),
        file("review.md"),
        summary("review.verdict.md"),
    );
    for count in ["2", "1", "1"] {
        b.next(&["ns", "gate2", "status=ready", &plan, &ps]);
        let c = format!("count={count}");
        b.next(&[
            "ns",
            "gate2",
            "status=findings",
            &review,
            &verdict,
            &c,
            "user=0",
        ]);
    }
    b.log_dump();
    b.reset("full", "2026-10-02", 2, "clean");
    b.next(&[
        "ns",
        "gate2",
        "status=findings",
        &review,
        &verdict,
        "count=0",
        "user=0",
    ]);
    b.next(&["ns", "gate2", "status=clean", &review, &verdict]);
    for t in ["question", "arch-gap", "split", "behavior-change"] {
        b.next(&["ns", "gate2", &format!("status={t}")]);
    }
    b.log_dump();
    b.finish("run-next-plan");
}

#[test]
fn it_next_gate2_stops_unless_fast_and_small() {
    let mut b = Board::new();
    let issue = file("issue.md");
    let new = ["ns", "gate2", "status=new", issue.as_str()];
    b.reset("full", "2026-10-02", 2, "clean");
    b.next(&new);
    b.reset("fast", "2026-10-02", 2, "clean");
    b.next(&new);
    b.next(&["ns", "gate2", "status=reuse", "issue=41", &issue]);
    b.reset("fast", "skipped", 1, "clean");
    b.next(&new);
    b.reset("fast", "2026-10-02", 3, "clean");
    b.next(&new);
    b.reset("fast", "2026-10-02", 2, "needs-user");
    b.next(&new);
    b.reset("-", "no-arch", 2, "clean");
    b.next(&new);
    b.finish("run-next-gate2");
}

#[test]
fn it_next_replan_is_pre_authorized_only_after_gate4() {
    let mut b = Board::new();
    let review = file("review.md");
    let clean = ["ns", "replan", "status=clean", review.as_str()];
    b.reset("fast", "2026-10-02", 2, "clean");
    b.log(&["plan-review=findings", "count=2", "round=1"]);
    b.log(&["stop", "kind=approval", "gate=gate4"]);
    b.log(&["replan", "from=s3"]);
    b.next(&[
        "ns",
        "replan",
        "status=ready",
        &file("plan.md"),
        &summary("plan.summary.md"),
    ]);
    b.next(&[
        "ns",
        "replan",
        "status=clean",
        &review,
        &summary("review.verdict.md"),
    ]);
    b.log_dump();
    b.reset("fast", "2026-10-02", 2, "clean");
    b.log(&["stop", "kind=stage"]);
    b.log(&["replan", "from=s3"]);
    b.next(&clean);
    b.reset("fast", "2026-10-02", 2, "clean");
    b.log(&["stop", "kind=approval", "gate=gate4"]);
    b.log(&["arch-rev=1"]);
    b.log(&["replan", "from=s3"]);
    b.artifact(
        "architecture.summary.md",
        "# ns — architecture summary (rev 2)\n",
    );
    b.next(&clean);
    b.reset("fast", "2026-10-02", 3, "clean");
    b.log(&["stop", "kind=approval", "gate=gate4"]);
    b.log(&["replan", "from=s3"]);
    b.next(&clean);
    b.reset("full", "2026-10-02", 3, "clean");
    b.log(&["stop", "kind=approval", "gate=gate4"]);
    b.log(&["replan", "from=s3"]);
    b.next(&clean);
    b.reset("fast", "2026-10-02", 2, "clean");
    b.log(&["stop", "kind=approval", "gate=gate4"]);
    b.log(&["replan", "from=s3"]);
    b.log(&["stop", "kind=stage"]);
    b.log(&["replan", "from=s5"]);
    b.next(&clean);
    b.finish("run-next-replan");
}

#[test]
fn it_next_branch_review_rounds() {
    let mut b = Board::new();
    let (review, verdict) = (file("review.md"), summary("review.verdict.md"));
    b.reset("full", "2026-10-02", 2, "clean");
    b.ok(&["board", "new", "ns.s1", "--state", "done"]);
    b.ok(&["board", "new", "ns.s2", "--state", "inreview"]);
    let findings = |count: &'static str, user: &'static str| {
        [
            "ns",
            "gate3",
            "status=findings",
            review.as_str(),
            verdict.as_str(),
            count,
            user,
        ]
    };
    let mut first = findings("count=2", "user=0").to_vec();
    first.push("stage=[s2]");
    b.next(&first);
    b.next(&[
        "ns",
        "gate3",
        "status=clean",
        &review,
        &verdict,
        "count=0",
        "user=0",
        "stage=[s2]",
    ]);
    b.log_dump();
    // s2 reaches done: the same log on a board where it already is.
    b.reset("full", "2026-10-02", 2, "needs-user");
    b.ok(&["board", "new", "ns.s1", "--state", "done"]);
    b.ok(&["board", "new", "ns.s2", "--state", "done"]);
    b.log(&["review=2-open", "scope=branch"]);
    b.log(&["review=2-resolved"]);
    b.next(&findings("count=0", "user=1"));
    b.ok(&["board", "set", "ns", "lane=fast"]);
    b.next(&findings("count=0", "user=1"));
    b.log_dump();
    b.reset("full", "2026-10-02", 2, "clean");
    b.ok(&["board", "new", "ns.s1", "--state", "done"]);
    b.ok(&["board", "new", "ns.s2", "--state", "done"]);
    let clean = [
        "ns",
        "gate3",
        "status=clean",
        review.as_str(),
        verdict.as_str(),
    ];
    b.next(&findings("count=1", "user=0"));
    b.next(&clean);
    b.next(&findings("count=3", "user=0"));
    b.next(&clean);
    b.log_dump();
    b.finish("run-next-branch");
}

#[test]
fn it_next_pr_and_ci() {
    let mut b = Board::new();
    b.reset("full", "2026-10-02", 2, "clean");
    b.next(&[
        "ns",
        "gate4",
        "status=ready",
        &file("pr.md"),
        "follow-ups=1",
    ]);
    b.next(&[
        "ns",
        "gate4",
        "status=question",
        &file("question.md"),
        "count=1",
    ]);
    b.next(&["ns", "ci", "ci=1"]);
    b.next(&["ns", "ci", "ci=0"]);
    b.log_dump();
    b.finish("run-next-pr-ci");
}

#[test]
fn it_next_usage() {
    let mut b = Board::new();
    b.reset("full", "2026-10-02", 2, "clean");
    b.next(&[]);
    b.next(&["ns", "nowhere", "status=ready"]);
    b.next(&["ns", "gate4", "status=new", "file=x"]);
    b.next(&["nosuch", "gate2", "status=ready"]);
    b.finish("run-next-usage");
}
