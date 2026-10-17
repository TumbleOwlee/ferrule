//! Issue and PR bodies on the fixture board, with the workflow's expected
//! outputs.

mod golden;

use std::path::Path;
use std::process::Command;

use golden::{Run, fixtures};

/// The fixture board imported and its artifacts under `.ferrule/runs/`.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let board = fixtures().join("board");
    let got = ferrule(dir.path(), &["card", "import", board.to_str().unwrap()]).text();
    assert_eq!(got, "import=ok cards=13\nexit=0");
    copy_dir(&board.join("artifacts"), &dir.path().join(".ferrule/runs"));
    dir
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
        .env("FERRULE_NOW", "2026-10-05T09:00")
        .cwd(project)
}

fn run_dir(p: &Path, slug: &str) -> std::path::PathBuf {
    p.join(".ferrule/runs").join(slug)
}

/// A card-less copy of `from` with `edit` applied to one of its files.
fn variant(p: &Path, from: &str, to: &str, file: &str, old: &str, new: &str) {
    copy_dir(&run_dir(p, from), &run_dir(p, to));
    let f = run_dir(p, to).join(file);
    let text = std::fs::read_to_string(&f).unwrap();
    assert!(text.contains(old), "{old}");
    std::fs::write(&f, text.replace(old, new)).unwrap();
}

#[test]
fn it_issue_body() {
    let p = project();
    let p = p.path();
    let body = |args: &[&str], golden: &str| {
        let mut all = vec!["issue", "body"];
        all.extend_from_slice(args);
        ferrule(p, &all).golden(golden);
    };
    body(&["demo"], "issue-body-demo");
    body(&["nospec"], "issue-body-nospec");
    body(&["violated"], "issue-body-violated");
    body(&["nosuch"], "issue-body-missing");
    body(&["lb"], "issue-body-epic");
    body(&["lb", "--slice", "1"], "issue-body-slice-1");
    body(&["lb", "--slice", "2"], "issue-body-slice-2");
    body(&["lb", "--slice", "3"], "issue-body-slice-3");
    body(&["sl"], "issue-body-slice-run");
    body(&["lb", "--slice", "9"], "issue-body-slice-missing");
    body(&["mrc"], "issue-body-mrc");
    body(&["demo", "--status", "x.md"], "issue-body-status-gone");
}

#[test]
fn it_issue_body_refuses_stage_and_agent_names() {
    let p = project();
    let p = p.path();
    let today = "Today the user restarts the tab by hand.";
    for (i, (line, golden)) in [
        ("Today s3 restarts it.", Some("issue-body-refused")),
        (
            "Today the fe-architect restarts it.",
            Some("issue-body-refused-agent"),
        ),
        ("Today `s3` restarts it.", Some("issue-body-refused-code")),
        (
            "Today `x` restarts s3. See `y`.",
            Some("issue-body-refused-between"),
        ),
        ("Today `docs/s3.md` restarts it.", None),
    ]
    .into_iter()
    .enumerate()
    {
        let slug = format!("sref{i}");
        variant(p, "demo", &slug, "issue.md", today, line);
        let run = ferrule(p, &["issue", "body", &slug]);
        match golden {
            Some(g) => run.golden(g),
            None => assert!(
                run.text().ends_with("exit=0"),
                "a path in a code span is no stage ref"
            ),
        }
    }
}

#[test]
fn it_pr_body() {
    let p = project();
    let p = p.path();
    ferrule(p, &["pr", "body", "demo", "draft"]).golden("pr-body-draft");
    ferrule(p, &["pr", "body", "demo", "full"]).golden("pr-body-full");
    ferrule(p, &["pr", "body", "mrcpr", "full"]).golden("pr-body-mrcpr");
    ferrule(p, &["pr", "body", "mrcpr", "draft"]).golden("pr-body-mrcpr-draft");
    variant(
        p,
        "demo",
        "sref2",
        "pr.md",
        "retry with backoff",
        "retry with backoff as in s2",
    );
    ferrule(p, &["pr", "body", "sref2", "full"]).golden("pr-body-refused");
    variant(
        p,
        "demo",
        "vmulti",
        "plan.summary.md",
        "- s2 — demo TUI",
        "- s1, s2 — demo TUI",
    );
    let out = ferrule(p, &["pr", "body", "vmulti", "draft"]).text();
    assert_eq!(
        out.lines()
            .filter(|l| *l == "  - beyond unit tests: demo TUI")
            .count(),
        2,
        "a verification line naming two stages shows under both: {out}"
    );
    assert_eq!(
        ferrule(p, &["pr", "body", "demo", "neither"]).text(),
        "exit=2"
    );
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
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
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn it_pr_draft_lists_the_spec_changes_the_branch_commits() {
    let p = project();
    let p = p.path();
    git(p, &["init", "-q", "-b", "main"]);
    let req = p.join("docs/specs/modbus/requirements.md");
    std::fs::create_dir_all(req.parent().unwrap()).unwrap();
    std::fs::write(
        &req,
        "**MB-R-001** — A register has exactly five properties.\n",
    )
    .unwrap();
    git(p, &["add", "docs"]);
    git(p, &["commit", "-qm", "base"]);
    git(p, &["checkout", "-q", "-b", "feat/mrcpr"]);
    let append = |text: &str| {
        let mut t = std::fs::read_to_string(&req).unwrap();
        t.push_str(text);
        std::fs::write(&req, t).unwrap();
    };
    append(
        "**MB-R-294** — `Esc` opens the close-confirmation popup.\n**MB-R-301** — The error box tracks the inputs live.\n**MB-R-293** — The inputs are trimmed.\n**MB-R-292** — `Enter` is refused while an input fails.\n",
    );
    git(p, &["commit", "-qam", "branch"]);
    append("**MB-R-295** — Polling continues.\n");
    for (at, args) in [
        (
            "2026-10-16T20:31",
            &["MB-R-295", "added", "origin=impl", "--", "polls on"][..],
        ),
        (
            "2026-10-16T20:30",
            &[
                "MB-R-294",
                "changed",
                "quote=Yes",
                "origin=user",
                "--",
                "popup on Esc",
            ][..],
        ),
        (
            "2026-10-16T20:32",
            &[
                "MB-R-292",
                "changed",
                "quote=Keep it open.",
                "origin=user",
                "--",
                "refuse on Enter",
            ][..],
        ),
    ] {
        let mut all = vec!["card", "log", "mrcpr.s3", "spec-delta"];
        all.extend_from_slice(args);
        let got = ferrule(p, &all).env("FERRULE_NOW", at).text();
        assert_eq!(got, "exit=0", "{args:?}");
    }
    ferrule(p, &["pr", "body", "mrcpr", "draft"]).golden("pr-body-mrcpr-draft-live");
    assert_eq!(
        ferrule(p, &["card", "set", "mrcpr", "branch="]).text(),
        "exit=0"
    );
    ferrule(p, &["pr", "body", "mrcpr", "draft"]).golden("pr-body-mrcpr-draft-nobranch");
}
