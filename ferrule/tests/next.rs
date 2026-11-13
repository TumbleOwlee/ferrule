//! `gate next` replays the workflow's next-step scenarios; the expected
//! transcripts are the shell tests' with each script call replaced by the
//! ferrule command that took its place.

mod golden;

use std::fmt::Write as _;
use std::path::Path;

use golden::{Run, check, seed_card, seed_log, seed_set};

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

    /// A fresh `ns`: parent in inprogress with `arch`, a brief of `size`
    /// new entries (`-` for no brief), a plan summary with s0 and `rows`
    /// more stages, and a verdict.
    fn reset(&mut self, size: &str, arch: &str, rows: u32, verdict: &str) {
        let transcript = std::mem::take(&mut self.transcript);
        *self = Board::new();
        self.transcript = transcript;
        let arch = format!("arch={arch}");
        seed_card(
            self.root(),
            "ns",
            "inprogress",
            &["branch=feat/ns", "gate1=2026-10-01", &arch],
        );
        if size != "-" {
            self.brief(size.parse().unwrap());
        }
        let mut summary = format!(
            "# ns — plan summary (rev 1)\n\nDecide: {rows} stages.\n\n| Stage | What | Pins | Files | Stop |\n|---|---|---|---|---|\n"
        );
        for i in 0..=rows {
            let _ = writeln!(summary, "| s{i} | stage {i} | — | 1 (x) | — |");
        }
        self.artifact("plan.summary.md", &summary);
        self.verdict(verdict);
    }

    /// A brief of `n` new entries.
    fn brief(&self, n: usize) {
        let mut text = String::from("# ns — spec diff (rev 1)\n\n");
        for i in 1..=n {
            let _ = writeln!(text, "## R{i} — new · requirements\nEntry {i}.\n");
        }
        self.artifact("spec-diff.md", &text);
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
    b.reset("-", "2026-10-02", 2, "clean");
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
    b.reset("-", "2026-10-02", 2, "clean");
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

/// GA-R-122 — gate 2 after the combined early stop is a notice unless a
/// Needs user item is open or the run reuses an issue.
#[test]
fn it_next_gate2_notice_after_combined_stop() {
    let mut b = Board::new();
    let issue = file("issue.md");
    let new = ["ns", "gate2", "status=new", issue.as_str()];
    b.reset("2", "2026-10-02", 2, "clean");
    b.next(&new);
    b.next(&["ns", "gate2", "status=reuse", "issue=41", &issue]);
    b.reset("2", "2026-10-02", 2, "needs-user");
    b.next(&new);
    b.finish("run-next-gate2");
}

/// GA-R-122 — after a brief of 3 entries gate 2 is a notice, unless the plan
/// stop was asked for, a Needs user item is open or the run reuses an issue.
#[test]
fn it_gate2_after_combined_stop_is_notice() {
    let mut b = Board::new();
    let new = ["ns", "gate2", "status=new"];
    b.reset("3", "2026-10-02", 2, "clean");
    b.next(&new);
    b.next(&["ns", "gate2", "status=reuse", "issue=41"]);
    b.verdict("needs-user");
    b.next(&new);
    b.verdict("clean");
    seed_set(b.root(), "ns", &["plan-stop=yes"]);
    b.next(&new);
    let out = b.transcript.clone();
    for line in [
        "next=notice",
        "next=stop reason=reuse",
        "next=stop reason=needs-user",
        "next=stop reason=plan-stop",
    ] {
        assert_eq!(out.matches(line).count(), 1, "{line}: {out}");
    }
}

/// GA-R-123, GA-R-128 — a run without an architecture stop stops at gate 2.
#[test]
fn it_gate2_without_arch_stops() {
    let mut b = Board::new();
    let new = ["ns", "gate2", "status=new"];
    b.reset("1", "skipped", 1, "clean");
    b.next(&new);
    b.reset("-", "no-arch", 2, "clean");
    b.next(&new);
    let out = b.transcript.clone();
    assert_eq!(
        out.matches("next=stop reason=no-arch-stop").count(),
        2,
        "{out}"
    );
}

/// GA-R-117, S501 — a brief of more than 3 entries stops at gate 2 for its size.
#[test]
fn it_gate2_large_brief_stops_for_size() {
    let mut b = Board::new();
    b.reset("4", "2026-10-02", 2, "clean");
    b.next(&["ns", "gate2", "status=new"]);
    assert!(
        b.transcript.contains("next=stop reason=size"),
        "{}",
        b.transcript
    );
}

/// GA-R-126 — a run without a brief whose architecture has at most 3
/// changes gets a notice at gate 2.
#[test]
fn it_gate2_no_brief_small_arch_notice() {
    let mut b = Board::new();
    b.reset("-", "2026-10-02", 2, "clean");
    b.artifact("architecture.md", &components(3));
    b.next(&["ns", "gate2", "status=new"]);
    assert!(b.transcript.contains("next=notice"), "{}", b.transcript);
}

/// GA-R-127 — a run without a brief whose architecture has more than 3
/// changes stops at gate 2 for its size.
#[test]
fn it_gate2_no_brief_large_arch_stops() {
    let mut b = Board::new();
    b.reset("-", "2026-10-02", 2, "clean");
    b.artifact("architecture.md", &components(4));
    b.next(&["ns", "gate2", "status=new"]);
    assert!(
        b.transcript.contains("next=stop reason=size"),
        "{}",
        b.transcript
    );
}

/// An architecture with `n` new components.
fn components(n: usize) -> String {
    let mut text = String::from("## Components\n| Component | Change |\n|---|---|\n");
    for i in 0..n {
        let _ = writeln!(text, "| c{i} | new |");
    }
    text
}

/// GA-R-116 — a brief of 2 or 3 entries goes to the architect without a stop.
#[test]
fn it_gate1_small_brief_goes_to_arch() {
    for n in ["2", "3"] {
        let mut b = Board::new();
        b.reset(n, "-", 2, "clean");
        b.next(&["ns", "gate1", "status=ready"]);
        assert!(b.transcript.contains("next=arch"), "{}", b.transcript);
        assert!(!b.transcript.contains("next=stop"), "{}", b.transcript);
    }
}

/// GA-R-117, S501 — a brief of more than 3 entries stops at gate 1.
#[test]
fn it_gate1_large_brief_stops() {
    let mut b = Board::new();
    b.reset("4", "-", 2, "clean");
    b.next(&["ns", "gate1", "status=ready"]);
    assert!(b.transcript.contains("next=stop"), "{}", b.transcript);
    assert!(
        b.transcript
            .contains("run: ferrule prompt render ns gate1\n"),
        "{}",
        b.transcript
    );
}

/// GA-R-117 — an epic-sized brief stops at gate 1 whatever its entry count,
/// and a gate 1 question is relayed.
#[test]
fn it_gate1_epic_and_question() {
    let mut b = Board::new();
    b.reset("1", "-", 2, "clean");
    b.next(&["ns", "gate1", "status=ready", "size=epic"]);
    b.next(&["ns", "gate1", "status=question"]);
    let out = &b.transcript;
    assert!(
        out.contains("next=stop\nrun: ferrule prompt render ns gate1 --epic --reason"),
        "{out}"
    );
    assert!(out.contains("next=relay"), "{out}");
}

/// GA-R-118 — a brief of one entry, or a no-diff brief, skips the architecture.
#[test]
fn it_gate1_one_entry_brief_skips_arch() {
    let mut b = Board::new();
    b.reset("1", "-", 2, "clean");
    b.next(&["ns", "gate1", "status=ready"]);
    b.reset("-", "-", 2, "clean");
    b.artifact(
        "spec-diff.md",
        "# ns — spec diff (rev 1)\n\n## R1 — violated\nA.\n\n- why\n\n## R2 — violated\nB.\n\n- why\n",
    );
    b.next(&["ns", "gate1", "status=no-diff"]);
    let out = b.transcript.clone();
    assert_eq!(out.matches("next=plan").count(), 2, "{out}");
    assert_eq!(
        out.matches("run: ferrule board set ns arch=skipped")
            .count(),
        2,
        "{out}"
    );
}

#[test]
fn it_next_replan_is_pre_authorized_only_after_gate4() {
    let mut b = Board::new();
    let review = file("review.md");
    let clean = ["ns", "replan", "status=clean", review.as_str()];
    b.reset("-", "2026-10-02", 2, "clean");
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
    b.reset("-", "2026-10-02", 2, "clean");
    b.log(&["stop", "kind=stage"]);
    b.log(&["replan", "from=s3"]);
    b.next(&clean);
    b.reset("-", "2026-10-02", 2, "clean");
    b.log(&["stop", "kind=approval", "gate=gate4"]);
    b.log(&["arch-rev=1"]);
    b.log(&["replan", "from=s3"]);
    b.artifact(
        "architecture.summary.md",
        "# ns — architecture summary (rev 2)\n",
    );
    b.next(&clean);
    b.reset("-", "2026-10-02", 3, "clean");
    b.log(&["stop", "kind=approval", "gate=gate4"]);
    b.log(&["replan", "from=s3"]);
    b.next(&clean);
    b.reset("-", "2026-10-02", 2, "clean");
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
    b.reset("-", "2026-10-02", 2, "clean");
    seed_card(b.root(), "ns.s1", "done", &[]);
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
            "minors=0",
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
        "minors=0",
        "stage=[s2]",
    ]);
    b.log_dump();
    // s2 reaches done: the same log on a board where it already is.
    b.reset("-", "2026-10-02", 2, "needs-user");
    b.ok(&["board", "set", "ns", "lane=full"]);
    seed_card(b.root(), "ns.s1", "done", &[]);
    seed_card(b.root(), "ns.s2", "done", &[]);
    b.log(&["review=2-open", "scope=branch"]);
    b.log(&["review=2-resolved"]);
    b.next(&findings("count=0", "user=1"));
    b.ok(&["board", "set", "ns", "lane=fast"]);
    b.next(&findings("count=0", "user=1"));
    b.log_dump();
    b.reset("-", "2026-10-02", 2, "clean");
    seed_card(b.root(), "ns.s1", "done", &[]);
    seed_card(b.root(), "ns.s2", "done", &[]);
    let clean = [
        "ns",
        "gate3",
        "status=clean",
        review.as_str(),
        verdict.as_str(),
        "minors=0",
    ];
    b.next(&findings("count=1", "user=0"));
    b.next(&clean);
    b.next(&findings("count=3", "user=0"));
    b.next(&clean);
    b.log_dump();
    b.finish("run-next-branch");
}

/// GA-R-095, GA-R-097, GA-R-099, GA-R-101, GA-R-103, GA-E-016 — a stage review routes its
/// minors to a fix step before the stop, and logs the clean review.
#[test]
fn it_next_stage_review_routes_minors() {
    let mut b = Board::new();
    b.reset("-", "2026-10-02", 3, "clean");
    seed_card(b.root(), "ns.s1", "done", &[]);
    b.ok(&["board", "new", "ns.s2", "--state", "inreview"]);
    b.next(&["ns", "stage", "status=findings", "count=1", "minors=1"]);
    b.next(&["ns", "stage", "status=clean", "minors=2"]);
    b.log_dump();
    b.reset("-", "2026-10-02", 3, "clean");
    seed_card(b.root(), "ns.s1", "done", &[]);
    b.ok(&["board", "new", "ns.s2", "--state", "inreview"]);
    b.next(&["ns", "stage", "status=clean", "minors=0"]);
    b.next(&[
        "ns",
        "stage",
        "status=findings",
        "count=0",
        "minors=1",
        "user=0",
    ]);
    b.log_dump();
    b.finish("run-next-stage");
}

/// GA-R-096, GA-R-097, GA-R-101 — a branch review routes its minors to a fix step before the
/// stop or the PR, and a stage review never resets an open branch count.
#[test]
fn it_next_branch_review_routes_minors() {
    let mut b = Board::new();
    b.reset("-", "2026-10-02", 2, "clean");
    seed_card(b.root(), "ns.s1", "done", &[]);
    seed_card(b.root(), "ns.s2", "done", &[]);
    b.next(&["ns", "gate3", "status=clean", "minors=2"]);
    b.log_dump();
    b.reset("-", "2026-10-02", 2, "clean");
    seed_card(b.root(), "ns.s1", "done", &[]);
    b.ok(&["board", "new", "ns.s2", "--state", "inreview"]);
    b.log(&["review=2-open", "scope=branch"]);
    b.log(&["review=clean", "scope=stage"]);
    b.next(&["ns", "gate3", "status=clean", "minors=1"]);
    b.log_dump();
    b.reset("-", "2026-10-02", 2, "needs-user");
    seed_card(b.root(), "ns.s1", "done", &[]);
    seed_card(b.root(), "ns.s2", "done", &[]);
    b.ok(&["board", "set", "ns", "lane=fast"]);
    b.next(&[
        "ns",
        "gate3",
        "status=findings",
        "count=0",
        "user=1",
        "minors=1",
    ]);
    b.log_dump();
    b.finish("run-next-branch-minors");
}

#[test]
fn it_next_pr_and_ci() {
    let mut b = Board::new();
    b.reset("-", "2026-10-02", 2, "clean");
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
/// GA-R-100 — a clean line at `stage` and `gate3` without a numeric `minors=` is a usage error.
fn it_next_usage() {
    let mut b = Board::new();
    b.reset("-", "2026-10-02", 2, "clean");
    b.next(&[]);
    b.next(&["ns", "nowhere", "status=ready"]);
    b.next(&["ns", "gate4", "status=new", "file=x"]);
    b.next(&["nosuch", "gate2", "status=ready"]);
    b.next(&["ns", "ci", "ci=green"]);
    b.next(&["ns", "gate2", "status=new", "issue=1;x"]);
    // Findings without a count would read as clean.
    b.next(&["ns", "gate3", "status=findings"]);
    b.next(&["ns", "gate3", "status=findings", "count=two"]);
    b.next(&["ns", "stage", "status=clean"]);
    b.next(&["ns", "gate3", "status=clean"]);
    b.next(&["ns", "gate3", "status=findings", "count=0"]);
    b.next(&["ns", "stage", "status=clean", "minors=x"]);
    b.finish("run-next-usage");
}

/// The parent card's log, one line per entry.
fn log_of(b: &Board) -> String {
    let shown = b.ferrule(&["board", "show", "ns"]).output();
    let shown = String::from_utf8_lossy(&shown.stdout).into_owned();
    shown.split("---\n").nth(2).unwrap_or("").to_owned()
}

#[test]
fn it_next_retried_status_line_logs_and_counts_once() {
    let mut b = Board::new();
    let (review, verdict) = (file("review.md"), summary("review.verdict.md"));
    b.reset("-", "2026-10-02", 2, "clean");
    seed_card(b.root(), "ns.s1", "done", &[]);
    let g3 = [
        "gate",
        "next",
        "ns",
        "gate3",
        "status=findings",
        &review,
        &verdict,
        "count=2",
        "user=0",
    ];
    let first = b.ferrule(&g3).text();
    let again = b.ferrule(&g3).text();
    assert!(first.contains("logged: review=2-open"), "{first}");
    assert_eq!(
        again,
        first.replace("logged: review=2-open scope=branch\n", "")
    );
    assert_eq!(log_of(&b).matches("review=2-open").count(), 1);

    // gate2: a retried findings is the same round, so round 3 still comes
    // after two `revise` answers.
    b.reset("-", "2026-10-02", 2, "clean");
    let (plan, ps) = (file("plan.md"), summary("plan.summary.md"));
    let ready = ["gate", "next", "ns", "gate2", "status=ready", &plan, &ps];
    let find = [
        "gate",
        "next",
        "ns",
        "gate2",
        "status=findings",
        &review,
        &verdict,
        "count=1",
        "user=0",
    ];
    let mut said = Vec::new();
    for _ in 0..2 {
        b.ferrule(&ready).text();
        said.push(b.ferrule(&find).text());
        said.push(b.ferrule(&find).text());
    }
    assert!(said.iter().all(|t| t.contains("next=revise")), "{said:?}");
    b.ferrule(&ready).text();
    assert!(b.ferrule(&find).text().contains("next=issue"));
    assert!(b.ferrule(&find).text().contains("next=issue"));
    let log = log_of(&b);
    for round in 1..=3 {
        assert_eq!(
            log.matches(&format!("plan-review=findings count=1 round={round}"))
                .count(),
            1,
            "{log}"
        );
    }
}
