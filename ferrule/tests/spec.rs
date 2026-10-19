mod golden;

use std::path::Path;
use std::process::Command;

use golden::{Run, fixtures};

fn doc() -> String {
    fixtures().join("md/doc.md").display().to_string()
}

/// A project whose `docs/specs` is the fixture spec tree.
fn spec_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::os::unix::fs::symlink(fixtures().join("specs"), dir.path().join("docs/specs")).unwrap();
    dir
}

#[test]
fn it_sections_lists_headings_outside_fences() {
    Run::new(["spec", "sections", &doc()]).golden("list-sections");
}

#[test]
fn it_section_prints_each_heading_in_turn() {
    Run::new(["spec", "section", "## Alpha", "## Beta", &doc()]).golden("extract-section");
}

#[test]
fn it_section_missing_heading_fails() {
    Run::new(["spec", "section", "## Gamma", &doc()]).golden("extract-section-missing");
}

#[test]
fn it_section_prefix_matches_heading_start() {
    Run::new(["spec", "section", "--prefix", "## Al", "## Beta", &doc()])
        .golden("extract-section-prefix");
}

#[test]
fn it_section_prefix_missing_fails() {
    Run::new(["spec", "section", "--prefix", "## Zz", &doc()])
        .golden("extract-section-prefix-missing");
}

#[test]
fn it_id_prints_file_line_text() {
    Run::new(["spec", "id", "--dir", "specs", "MB-R-002", "MB-E-001"])
        .cwd(fixtures())
        .golden("extract-id");
}

#[test]
fn it_id_missing_fails() {
    let out = Run::new(["spec", "id", "--dir", "specs", "MB-R-999"])
        .cwd(fixtures())
        .output();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

#[test]
fn it_related_finds_keywords_in_all_three_entry_forms() {
    let project = spec_project();
    Run::new(["spec", "related", "polls", "slow"])
        .env("CLAUDE_PROJECT_DIR", project.path())
        .golden("related-ids");
}

#[test]
fn it_related_without_hit_says_none() {
    let project = spec_project();
    Run::new(["spec", "related", "nosuchword"])
        .env("CLAUDE_PROJECT_DIR", project.path())
        .golden("related-ids-none");
}

#[test]
fn it_related_keywords_are_literal() {
    let project = spec_project();
    Run::new(["spec", "related", "retr.ed"])
        .env("CLAUDE_PROJECT_DIR", project.path())
        .golden("related-ids-literal");
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
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
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn append(path: &Path, text: &str) {
    let mut s = std::fs::read_to_string(path).unwrap_or_default();
    s.push_str(text);
    std::fs::write(path, s).unwrap();
}

#[test]
fn it_collisions_reports_ids_both_branches_took() {
    let repo = tempfile::tempdir().unwrap();
    let wt = repo.path();
    let spec = wt.join("docs/specs/r.md");
    git(wt, &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(spec.parent().unwrap()).unwrap();
    append(&spec, "**MB-R-001** — The client connects.\n");
    git(wt, &["add", "."]);
    git(wt, &["commit", "-q", "-m", "base"]);
    git(wt, &["checkout", "-q", "-b", "feat/x"]);
    append(
        &spec,
        "**MB-R-002** — The client polls.\n**MB-R-003** — The client retries.\n",
    );
    git(wt, &["commit", "-q", "-am", "x"]);

    let run = || Run::new(["spec", "collisions", &wt.display().to_string(), "main"]).text();
    assert_eq!(run(), "ids=clean\nexit=0");

    git(wt, &["checkout", "-q", "main"]);
    append(&spec, "**MB-R-002** — The server answers.\n");
    git(wt, &["commit", "-q", "-am", "y"]);
    git(wt, &["checkout", "-q", "feat/x"]);
    assert_eq!(
        run(),
        "ids=collide MB-R-002 hint=renumber these IDs on the branch\nexit=1"
    );
}

#[test]
fn it_collisions_defaults_to_the_configured_main_branch() {
    let repo = tempfile::tempdir().unwrap();
    let wt = repo.path();
    let spec = wt.join("docs/specs/r.md");
    git(wt, &["init", "-q", "-b", "trunk"]);
    std::fs::create_dir_all(wt.join(".ferrule")).unwrap();
    std::fs::write(
        wt.join(".ferrule/ferrule.toml"),
        "main_branch = \"trunk\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(spec.parent().unwrap()).unwrap();
    append(&spec, "**MB-R-001** — The client connects.\n");
    git(wt, &["add", "."]);
    git(wt, &["commit", "-q", "-m", "base"]);
    git(wt, &["checkout", "-q", "-b", "feat/x"]);
    append(&spec, "**MB-R-002** — The client polls.\n");
    git(wt, &["commit", "-q", "-am", "x"]);
    git(wt, &["checkout", "-q", "trunk"]);
    append(&spec, "**MB-R-002** — The server answers.\n");
    git(wt, &["commit", "-q", "-am", "y"]);
    git(wt, &["update-ref", "refs/remotes/origin/trunk", "trunk"]);
    git(wt, &["checkout", "-q", "feat/x"]);

    let out = Run::new(["spec", "collisions", &wt.display().to_string()])
        .cwd(wt)
        .text();
    assert_eq!(
        out,
        "ids=collide MB-R-002 hint=renumber these IDs on the branch\nexit=1"
    );
}

#[test]
fn it_collisions_without_merge_base_is_an_error() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    let out = Run::new([
        "spec",
        "collisions",
        &repo.path().display().to_string(),
        "nosuch",
    ])
    .text();
    assert_eq!(
        out,
        "ids=error reason=no-merge-base hint=fetch the REF, or pass a REF the branch descends from\nexit=2"
    );
}

fn run_diff(slug: &str) -> String {
    fixtures()
        .join("board/artifacts")
        .join(slug)
        .join("spec-diff.md")
        .display()
        .to_string()
}

#[test]
fn it_render_each_part_of_a_spec_diff() {
    for part in [
        "scenarios",
        "requirements",
        "entries",
        "changes",
        "deferred",
    ] {
        Run::new(["spec", "render", &run_diff("demo"), part])
            .golden(&format!("spec-render-{part}"));
    }
}

#[test]
fn it_render_only_keeps_named_ids_and_uncited_rows_with_first() {
    Run::new([
        "spec",
        "render",
        &run_diff("lb"),
        "entries",
        "--only",
        "R1",
        "--with-unnumbered",
    ])
    .golden("spec-render-entries-only");
}

#[test]
fn it_render_only_without_first_drops_uncited_rows() {
    Run::new(["spec", "render", &run_diff("lb"), "entries", "--only", "R3"])
        .golden("spec-render-lb-entries-only-nofirst");
}

#[test]
fn it_render_only_matches_local_or_fixed_ids() {
    Run::new([
        "spec",
        "render",
        &run_diff("mrcpr"),
        "entries",
        "--only",
        "R2 MB-E-105",
    ])
    .golden("spec-render-entries-only-fixed");
    Run::new([
        "spec",
        "render",
        &run_diff("lb"),
        "scenarios",
        "--only",
        "S1 S3",
    ])
    .golden("spec-render-scenarios-only");
}

#[test]
fn it_render_writes_fixed_ids_back_as_local_ids() {
    Run::new(["spec", "render", &run_diff("mrcpr"), "requirements"])
        .golden("spec-render-mrcpr-requirements");
    Run::new(["spec", "render", &run_diff("mrcpr"), "changes"]).golden("spec-render-mrcpr-changes");
}

#[test]
fn it_render_pr_form_marks_requirements_reworded_after_filing() {
    let p = tempfile::tempdir().unwrap();
    let board = fixtures().join("board");
    let imported = Run::new(["card", "import", board.to_str().unwrap()])
        .env("CLAUDE_PROJECT_DIR", p.path())
        .cwd(p.path())
        .text();
    assert_eq!(imported, "import=ok cards=13\nexit=0");
    Run::new([
        "spec",
        "render",
        &run_diff("mrcpr"),
        "entries",
        "--issue-ids",
    ])
    .env("CLAUDE_PROJECT_DIR", p.path())
    .cwd(p.path())
    .golden("spec-render-issue-ids");
}

#[test]
fn it_render_violated_requirements_lead_with_no_spec_change() {
    Run::new(["spec", "render", &run_diff("violated"), "requirements"])
        .golden("spec-render-violated");
    Run::new(["spec", "render", &run_diff("violated"), "entries"])
        .golden("spec-render-violated-entries");
}

#[test]
fn it_render_usage_and_missing_file() {
    Run::new(["spec", "render"]).golden("spec-render-usage");
    Run::new(["spec", "render", "nosuch.md", "scenarios"]).golden("spec-render-missing");
}
