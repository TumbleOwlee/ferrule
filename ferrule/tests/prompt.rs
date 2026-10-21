//! The prompt group on the fixture board, with the workflow's expected
//! outputs.

mod golden;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use golden::{Run, check, fixtures};

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
    render(p, &["demo", "gate3"]).golden("prompt-render-gate3");
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
    render(p, &["demo", "delta", "--stage", "s1"]).golden("prompt-render-delta");
    render(p, &["demo", "auto", "--stage", "s1"]).golden("prompt-render-auto");
    render(p, &["demo", "gate2-notice", "--issue", "new"]).golden("prompt-render-gate2-notice");
    render(p, &["demo", "replan-notice", "--from", "s3"]).golden("prompt-render-replan-notice");
    render(p, &["lb", "epic"]).golden("prompt-render-epic");
    check("prompt-render-stops", &stops(p));
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
    set("lane=fast");
    render(p, &["demo", "arch"]).golden("prompt-render-arch-fast");
    let plan = art(p, "demo", "plan.summary.md");
    let mut text = std::fs::read_to_string(&plan).unwrap();
    text.extend((1..=40).map(|i| format!("- line {i}\n")));
    std::fs::write(&plan, text).unwrap();
    assert_eq!(render(p, &["demo", "gate2"]).text(), "exit=3");
}

#[test]
fn it_render_without_a_viewer_names_the_files() {
    let p = project();
    let p = p.path();
    config(p, "");
    let out = render(p, &["demo", "gate3"]).text();
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
