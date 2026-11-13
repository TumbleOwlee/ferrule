//! The prompt group on the fixture board, with the workflow's expected
//! outputs.

mod golden;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use golden::{Run, check, fixtures, seed_log, seed_set};

const NOW: &str = "2026-10-05T09:00";

/// The fixture board imported, its artifacts under `.ferrule/runs/`, and
/// a viewer that always succeeds.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let board = fixtures().join("board");
    let got = ferrule(dir.path(), &["board", "import", board.to_str().unwrap()]).text();
    assert_eq!(got, "import=ok cards=13\nexit=0");
    copy_dir(&board.join("artifacts"), &dir.path().join(".ferrule/runs"));
    config(dir.path(), "[viewer]\ncommand = [\"true\"]\n");
    dir
}

fn config(p: &Path, text: &str) {
    std::fs::write(p.join(".ferrule/ferrule.toml"), text).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn ferrule(project: &Path, args: &[&str]) -> Run {
    Run::new(args)
        .env("CLAUDE_PROJECT_DIR", project)
        .env("FERRULE_NOW", NOW)
        .cwd(project)
}

fn art(p: &Path, slug: &str, name: &str) -> PathBuf {
    p.join(".ferrule/runs").join(slug).join(name)
}

fn lines(p: &Path, files: &[PathBuf], extra: &[&str]) -> Run {
    let mut args: Vec<String> = vec!["prompt".into(), "lines".into()];
    args.extend(extra.iter().map(|&s| s.to_owned()));
    args.extend(files.iter().map(|f| f.display().to_string()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    ferrule(p, &args)
}

#[test]
fn it_lines_per_file() {
    let p = project();
    let p = p.path();
    let a = |n: &str| art(p, "demo", n);
    lines(p, &[a("spec-diff.md")], &[]).golden("prompt-lines-spec-diff");
    lines(p, &[a("decisions.md")], &[]).golden("prompt-lines-decisions");
    lines(p, &[a("decisions.md")], &["--seen", "7"]).golden("prompt-lines-decisions-seen");
    lines(p, &[a("architecture.summary.md")], &[]).golden("prompt-lines-architecture");
    lines(p, &[a("plan.summary.md")], &[]).golden("prompt-lines-plan");
    lines(p, &[a("review.verdict.md")], &[]).golden("prompt-lines-verdict");
    lines(
        p,
        &[
            a("issue.md"),
            a("split-issue.md"),
            a("pr.md"),
            a("follow-up-1.md"),
        ],
        &[],
    )
    .golden("prompt-lines-titles");
    lines(p, &[a("question.md")], &[]).golden("prompt-lines-question");
    lines(p, &[art(p, "violated", "spec-diff.md")], &[]).golden("prompt-lines-violated");
    lines(p, &[art(p, "lb", "slices.md")], &[]).golden("prompt-lines-slices");
    let s = art(p, "lb", "slices.md");
    let text = std::fs::read_to_string(&s)
        .unwrap()
        .replace("Entries: R3\n", "Entries: none\n")
        .replace("Entries: R1\n", "Entries: R1, E1\n");
    std::fs::write(&s, text).unwrap();
    lines(p, &[s], &[]).golden("prompt-lines-slices-gaps");
    lines(p, &[a("nosuch.md")], &[]).golden("prompt-lines-unreadable");
}

#[test]
fn it_show_records_the_marker_and_caps_summaries() {
    let p = project();
    let p = p.path();
    config(p, "");
    let (plan, verdict) = (
        art(p, "demo", "plan.summary.md"),
        art(p, "demo", "review.verdict.md"),
    );
    let out = ferrule(
        p,
        &[
            "prompt",
            "show",
            "--no-lines",
            plan.to_str().unwrap(),
            verdict.to_str().unwrap(),
        ],
    )
    .text();
    let root = p.canonicalize().unwrap();
    let out = out.replace(root.to_str().unwrap(), "<P>");
    let mut want = format!(
        "prompt show: stdout: <P>/.ferrule/runs/demo/plan.summary.md (approval marker: demo)\n{}",
        std::fs::read_to_string(&plan).unwrap()
    );
    let _ = write!(
        want,
        "prompt show: stdout: <P>/.ferrule/runs/demo/review.verdict.md (approval marker: demo)\n{}exit=0",
        std::fs::read_to_string(&verdict).unwrap()
    );
    assert_eq!(out, want);

    config(p, "[viewer]\ncommand = [\"true\"]\n");
    let out = ferrule(p, &["prompt", "show", plan.to_str().unwrap()]).text();
    let out = out.replace(root.to_str().unwrap(), "<P>");
    assert!(
        out.starts_with("prompt show: opened with true: <P>/.ferrule/runs/demo/plan.summary.md (approval marker: demo)\nPlan: 3 stages"),
        "{out}"
    );

    config(p, "[viewer]\ncommand = [\"false\"]\n");
    let out = ferrule(p, &["prompt", "show", plan.to_str().unwrap()]).text();
    assert!(out.ends_with("exit=1"), "{out}");

    let mut text = std::fs::read_to_string(&plan).unwrap();
    text.extend((1..=40).map(|i| format!("- line {i}\n")));
    std::fs::write(&plan, text).unwrap();
    assert_eq!(
        ferrule(p, &["prompt", "show", plan.to_str().unwrap()]).text(),
        "exit=3"
    );
    assert_eq!(
        ferrule(
            p,
            &[
                "prompt",
                "show",
                art(p, "demo", "nosuch.md").to_str().unwrap()
            ]
        )
        .text(),
        "exit=1"
    );
}

fn render(p: &Path, args: &[&str]) -> Run {
    let mut all = vec!["prompt", "render"];
    all.extend_from_slice(args);
    ferrule(p, &all)
}

fn stops(p: &Path) -> String {
    let shown = ferrule(p, &["board", "show", "demo"]).output();
    let shown = String::from_utf8_lossy(&shown.stdout).into_owned();
    let mut out = String::new();
    for line in shown.lines().filter(|l| l.contains(" stop ")) {
        out.push_str(line.split_once(' ').map_or(line, |(_, b)| b));
        out.push('\n');
    }
    out.push_str("exit=0");
    out
}

#[test]
fn it_render_every_stop() {
    let p = project();
    let p = p.path();
    render(p, &["demo", "gate1"]).golden("prompt-render-gate1");
    render(
        p,
        &[
            "demo",
            "gate1",
            "--epic",
            "--reason",
            "two areas, 14 entries",
        ],
    )
    .golden("prompt-render-gate1-epic");
    render(p, &["demo", "arch"]).golden("prompt-render-arch");
    render(p, &["demo", "gate2", "--issue", "new"]).golden("prompt-render-gate2");
    render(
        p,
        &[
            "demo",
            "stage",
            "--stage",
            "s1",
            "--reason",
            "verification",
            "--detail",
            "demo TUI",
        ],
    )
    .golden("prompt-render-stage");
    render(p, &["demo", "replan", "--from", "s2"]).golden("prompt-render-replan");
    render(p, &["demo", "gate4"]).golden("prompt-render-gate4");
    render(
        p,
        &[
            "demo",
            "question",
            "--where",
            "gate1",
            "--agent",
            "fe-author",
        ],
    )
    .golden("prompt-render-question");
    render(
        p,
        &[
            "demo",
            "blocked",
            "--stage",
            "s2",
            "--reason",
            "the parse_frame reference names no signature",
        ],
    )
    .golden("prompt-render-blocked");
    seed_log(p, "2026-10-05T09:00", "demo", &["pushed=bbb2222"]);
    render(p, &["demo", "auto", "--stage", "s1"]).golden("prompt-render-auto");
    render(p, &["demo", "gate2-notice", "--issue", "new"]).golden("prompt-render-gate2-notice");
    render(p, &["demo", "replan-notice", "--from", "s3"]).golden("prompt-render-replan-notice");
    render(p, &["lb", "epic"]).golden("prompt-render-epic");
    check("prompt-render-stops", &stops(p));
}

/// GA-R-133, GA-E-025 — `prompt render` has no `delta` stop.
#[test]
fn it_prompt_render_has_no_delta_stop() {
    let p = project();
    assert_eq!(
        render(p.path(), &["demo", "delta", "--stage", "s1"]).text(),
        "exit=2"
    );
}

/// GA-R-136 — `prompt render` has no `gate3` stop; nothing is logged.
#[test]
fn it_prompt_render_has_no_gate3_stop() {
    let p = project();
    let before = ferrule(p.path(), &["board", "show", "demo"]).text();
    assert_eq!(render(p.path(), &["demo", "gate3"]).text(), "exit=2");
    assert_eq!(ferrule(p.path(), &["board", "show", "demo"]).text(), before);
}

/// GW-R-015, GW-R-016, GW-R-017, GW-R-024, GW-R-027 — the diverged stop shows both heads, three options, no stage.
#[test]
fn it_render_blocked_diverged() {
    let p = project();
    let p = p.path();
    render(
        p,
        &[
            "demo", "blocked", "--local", "abc1234", "--remote", "def5678",
        ],
    )
    .golden("prompt-render-blocked-diverged");
    let shown = ferrule(p, &["board", "show", "demo"]).text();
    assert!(shown.contains(" stop kind=blocked"), "{shown}");
}

/// GW-R-083, GW-R-085, GW-R-086, GW-R-087, GW-R-089 — the sha stop shows the mismatch, the stays-refused line and three options, no stage.
#[test]
fn it_render_blocked_sha() {
    let p = project();
    let p = p.path();
    render(p, &["demo", "blocked", "--mismatch", "card"]).golden("prompt-render-blocked-sha");
    let shown = ferrule(p, &["board", "show", "demo"]).text();
    assert!(shown.contains(" stop kind=blocked"), "{shown}");
}

/// GW-R-084 — with every stage card committed the sha stop omits `a` and keeps the letters.
#[test]
fn it_render_blocked_sha_omits_continue_with_no_stage_left() {
    let p = project();
    let p = p.path();
    let check = |slug: &str| {
        let got = render(p, &[slug, "blocked", "--mismatch", "card"]).text();
        assert!(!got.contains("a) continue"), "{got}");
        assert!(
            got.contains("- b) fixed outside ferrule, push again"),
            "{got}"
        );
        assert!(got.contains("- c) stop"), "{got}");
        assert!(got.contains("Answer: <1: b | c>"), "{got}");
    };
    check("mrcpr");
    let at = "2026-10-05T09:00";
    seed_log(p, at, "demo.s1", &["commit=abc1234"]);
    seed_log(p, at, "demo.s2", &["commit=def5678"]);
    check("demo");
}

/// GW-R-088 — `--mismatch` with `--local` or `--remote` is a usage error.
#[test]
fn it_render_blocked_sha_refuses_a_head() {
    let p = project();
    let p = p.path();
    for heads in [
        &["--local", "abc1234"][..],
        &["--remote", "def5678"],
        &["--local", "abc1234", "--remote", "def5678"],
    ] {
        let mut args = vec!["demo", "blocked", "--mismatch", "card"];
        args.extend_from_slice(heads);
        let got = render(p, &args).text();
        assert!(got.ends_with("exit=2"), "{got}");
    }
}

/// GW-R-066 — the auto-continue notice takes its commit from a push and its
/// coverage from a pass, each logged after the run's last rebase.
#[test]
fn it_render_auto_shows_no_pass_from_before_a_rebase() {
    let p = project();
    let p = p.path();
    let at = "2026-10-05T09:00";
    let notice = || render(p, &["demo", "auto", "--stage", "s1"]).text();
    seed_log(p, at, "demo", &["pushed=bbb2222"]);
    assert!(notice().contains("gauntlet pass 81.2% · pushed bbb2222"));
    seed_log(
        p,
        at,
        "demo",
        &["rebase=ccc3333", "base=origin/main", "from=bbb2222"],
    );
    assert!(notice().contains("gauntlet pass - · pushed -"));
    seed_log(
        p,
        at,
        "demo",
        &["gauntlet=fail", "step=test", "sha=ccc3333"],
    );
    assert!(notice().contains("gauntlet pass - · pushed -"));
    seed_log(
        p,
        at,
        "demo",
        &["gauntlet=pass", "cov=90.0%", "sha=ccc3333"],
    );
    assert!(notice().contains("gauntlet pass 90.0% · pushed -"));
    seed_log(p, at, "demo", &["pushed=ccc3333"]);
    assert!(notice().contains("gauntlet pass 90.0% · pushed ccc3333"));
}

/// GA-R-098, GA-R-102, GA-E-017 — a stop and the auto-continue notice state the minors the
/// newest review left, until a stop is logged after it.
#[test]
fn it_render_states_the_minors_fixed() {
    let p = project();
    let p = p.path();
    let at = "2026-10-05T09:00";
    seed_log(p, at, "demo", &["review=clean", "scope=stage", "minors=2"]);
    let auto = render(p, &["demo", "auto", "--stage", "s1"]).text();
    assert!(auto.contains("· minors fixed 2 · next:"), "{auto}");
    let stage = render(
        p,
        &["demo", "stage", "--stage", "s1", "--reason", "plan-marked"],
    )
    .text();
    assert!(stage.contains("Minors fixed after the review, not re-reviewed: 2"));
    let gate4 = render(p, &["demo", "gate4"]).text();
    assert!(!gate4.contains("Minors fixed"), "{gate4}");
    let after_stop = render(p, &["demo", "auto", "--stage", "s1"]).text();
    assert!(!after_stop.contains("minors fixed"), "{after_stop}");
    seed_log(p, at, "demo", &["review=clean", "minors=3"]);
    let gate4 = render(p, &["demo", "gate4"]).text();
    assert!(gate4.contains("Minors fixed after the review, not re-reviewed: 3"));
    seed_log(p, at, "demo", &["review=clean", "scope=stage"]);
    let none = render(p, &["demo", "auto", "--stage", "s1"]).text();
    assert!(!none.contains("minors fixed"), "{none}");
}

/// GW-R-028 — one head alone is a usage error; both or neither render.
#[test]
fn it_render_blocked_takes_both_heads_or_neither() {
    let p = project();
    let p = p.path();
    assert!(
        render(p, &["demo", "blocked", "--local", "abc1234"])
            .text()
            .ends_with("exit=2")
    );
    assert!(
        render(p, &["demo", "blocked", "--remote", "def5678"])
            .text()
            .ends_with("exit=2")
    );
    render(
        p,
        &[
            "demo",
            "blocked",
            "--stage",
            "s2",
            "--reason",
            "the parse_frame reference names no signature",
        ],
    )
    .golden("prompt-render-blocked");
}

/// GA-R-005, GA-R-006, GA-R-007 — the stage stop names what follows the approval;
/// at a batch's review point it commits the batch's review fixes.
#[test]
fn it_render_stage_says_what_follows() {
    let p = project();
    let p = p.path();
    let middle = render(
        p,
        &["demo", "stage", "--stage", "s1", "--reason", "plan-marked"],
    )
    .text();
    let last = render(
        p,
        &["demo", "stage", "--stage", "s2", "--reason", "plan-marked"],
    )
    .text();
    assert!(middle.contains(
        "Approving will:\n- commit s1, push, update the draft PR\n- spawn the next stage\nAnswer: approve · change <what> · stop"
    ));
    assert!(last.contains(
        "Approving will:\n- commit s2, push, update the draft PR\n- next stop: gate 4\nAnswer: approve · change <what> · stop"
    ));
    let general = render(p, &["demo", "stage", "--reason", "plan-marked"]).text();
    assert!(general.contains("- spawn the next stage\nAnswer: approve · change <what> · stop"));
    assert!(!general.contains("gate 4"));
    seed_set(p, "demo.s1", &["tier=batched"]);
    seed_set(p, "demo.s2", &["tier=batched"]);
    let batch = render(
        p,
        &["demo", "stage", "--stage", "s2", "--reason", "plan-marked"],
    )
    .text();
    assert!(batch.contains(
        "Approving will:\n- commit the review fixes of s1, s2, push, update the draft PR\n- next stop: gate 4"
    ));
    let open = render(
        p,
        &["demo", "stage", "--stage", "s1", "--reason", "plan-marked"],
    )
    .text();
    assert!(open.contains("- commit s1, push, update the draft PR\n- spawn the next stage"));
}

#[test]
fn it_render_approved_records_what_was_shown() {
    let p = project();
    let p = p.path();
    assert!(render(p, &["demo", "gate2"]).text().ends_with("exit=0"));
    render(p, &["demo", "approved"]).golden("prompt-render-approved");
    render(p, &["demo", "arch"]).golden("prompt-render-since-rev");
    let set = |kv: &str| assert_eq!(ferrule(p, &["board", "set", "demo", kv]).text(), "exit=0");
    set("gate1=");
    render(p, &["demo", "arch"]).golden("prompt-render-arch-combined");
    let plan = art(p, "demo", "plan.summary.md");
    let mut text = std::fs::read_to_string(&plan).unwrap();
    text.extend((1..=40).map(|i| format!("- line {i}\n")));
    std::fs::write(&plan, text).unwrap();
    assert_eq!(render(p, &["demo", "gate2"]).text(), "exit=3");
}

/// GA-R-125, S505 — a Needs user item the branch review left is shown,
/// lettered, on the gate 4 prompt.
#[test]
fn it_gate4_prompt_letters_needs_user() {
    let p = project();
    let p = p.path();
    let out = render(p, &["demo", "gate4"]).text();
    assert!(out.contains("Needs user:\n- MB-R-161"), "{out}");
    assert!(out.contains("  - a) revert to retry forever"), "{out}");
    assert!(out.contains("  - b) keep the 1 hour limit"), "{out}");
}

/// GA-R-129 — the combined early stop's architecture prompt offers a stop at
/// the plan too; separate early stops do not.
#[test]
fn it_arch_prompt_offers_plan_stop() {
    let p = project();
    let p = p.path();
    let set = |kv: &str| assert_eq!(ferrule(p, &["board", "set", "demo", kv]).text(), "exit=0");
    set("gate1=");
    let brief = |n: usize| {
        let text = "## R1 — new · requirements\nEntry.\n\n".repeat(n);
        std::fs::write(
            art(p, "demo", "spec-diff.md"),
            format!("# demo — spec diff (rev 1)\n\n{text}"),
        )
        .unwrap();
    };
    brief(3);
    assert!(
        render(p, &["demo", "arch"])
            .text()
            .contains("stop at the plan too")
    );
    brief(4);
    assert!(
        !render(p, &["demo", "arch"])
            .text()
            .contains("stop at the plan too")
    );
}

/// GA-R-130 — `prompt render` has no `--recommend` flag.
#[test]
fn it_prompt_render_has_no_recommend_flag() {
    let p = project();
    let p = p.path();
    assert_eq!(
        render(p, &["demo", "gate1", "--recommend", "full"]).text(),
        "exit=2"
    );
}

#[test]
fn it_render_without_a_viewer_names_the_files() {
    let p = project();
    let p = p.path();
    config(p, "");
    let out = render(
        p,
        &["demo", "stage", "--stage", "s1", "--reason", "plan-marked"],
    )
    .text();
    let root = p.canonicalize().unwrap();
    let out = out.replace(root.to_str().unwrap(), "<P>");
    assert_eq!(
        out.lines().nth(1),
        Some("Viewer: none available — open manually: <P>/.ferrule/runs/demo/review.verdict.md")
    );
}

/// GA-R-114 — a viewer that cannot start falls back to the file on stdout.
#[test]
fn it_show_prints_the_file_when_the_viewer_cannot_start() {
    let p = project();
    let p = p.path();
    config(p, "[viewer]\ncommand = [\"ferrule-no-such-viewer\"]\n");
    let plan = art(p, "demo", "plan.summary.md");
    let out = ferrule(p, &["prompt", "show", "--no-lines", plan.to_str().unwrap()]).text();
    assert!(out.starts_with("prompt show: stdout: "), "{out}");
    assert!(
        out.contains(&std::fs::read_to_string(&plan).unwrap()),
        "{out}"
    );
    assert!(out.ends_with("exit=0"), "{out}");
}

/// GA-R-114 — render treats a viewer that cannot start as no viewer.
#[test]
fn it_render_without_a_startable_viewer_names_the_files() {
    let p = project();
    let p = p.path();
    config(p, "[viewer]\ncommand = [\"ferrule-no-such-viewer\"]\n");
    let out = render(
        p,
        &["demo", "stage", "--stage", "s1", "--reason", "plan-marked"],
    )
    .text();
    let root = p.canonicalize().unwrap();
    let out = out.replace(root.to_str().unwrap(), "<P>");
    assert_eq!(
        out.lines().nth(1),
        Some("Viewer: none available — open manually: <P>/.ferrule/runs/demo/review.verdict.md")
    );
}

#[test]
fn it_render_usage() {
    let p = project();
    let p = p.path();
    assert_eq!(render(p, &["demo", "nope"]).text(), "exit=2");
    assert_eq!(render(p, &["demo", "gate1", "loose"]).text(), "exit=2");
    assert_eq!(
        render(p, &["nosuch", "gate1"]).text(),
        "render=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
    assert_eq!(render(p, &["demo", "auto"]).text(), "exit=2");
}

#[test]
fn it_reading_without_a_board_creates_none() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join(".ferrule")).unwrap();
    config(p, "[viewer]\ncommand = [\"true\"]\n");
    let note = p.join("note.md");
    std::fs::write(&note, "# Note\n").unwrap();
    let shown = ferrule(p, &["prompt", "show", note.to_str().unwrap()]).text();
    assert!(shown.ends_with("exit=0"), "{shown}");
    let gauntlet = ferrule(p, &["ci", "gauntlet", "demo"]).text();
    assert!(
        gauntlet.starts_with("gauntlet=refused reason=card"),
        "{gauntlet}"
    );
    let cleanup = ferrule(p, &["run", "cleanup", "demo"]).text();
    assert!(
        cleanup.starts_with("cleanup=refused reason=card"),
        "{cleanup}"
    );
    assert!(!p.join(".ferrule/state.db").exists());
}

#[test]
fn it_show_says_why_the_viewer_failed() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::create_dir_all(p.join(".ferrule")).unwrap();
    config(
        p,
        "[viewer]\ncommand = [\"sh\", \"-c\", \"echo no display >&2; exit 1\", \"sh\"]\n",
    );
    let note = p.join("note.md");
    std::fs::write(&note, "# Note\n").unwrap();
    let out = ferrule(p, &["prompt", "show", note.to_str().unwrap()]).output();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("prompt show: sh: no display"), "{err}");
    assert_eq!(out.status.code(), Some(1));
}
