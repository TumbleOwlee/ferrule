//! `ferrule guide`: topics cut to the run's card, project overrides, the
//! soundness check.

mod golden;

use std::path::Path;

use golden::{Run, seed_log};

fn ferrule(root: &Path, cwd: &Path, args: &[&str]) -> String {
    Run::new(args)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("FERRULE_NOW", "2026-10-05T09:00")
        .cwd(cwd)
        .text()
}

fn ok(root: &Path, args: &[&str]) -> String {
    let out = ferrule(root, root, args);
    assert!(out.ends_with("exit=0"), "{args:?}: {out}");
    out
}

#[test]
fn it_guide_cuts_to_the_cards_lane_and_state() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["board", "new", "demo", "lane=fast", "pr=34"]);

    let stages = ok(root, &["guide", "stages", "--slug", "demo"]);
    assert!(stages.contains("## Fast lane\n"), "{stages}");
    assert!(!stages.contains("## Full lane"), "{stages}");
    let run_dir = root.join(".ferrule/runs/demo").display().to_string();
    assert!(stages.contains(&format!("`{run_dir}/plan.md`")), "{stages}");
    assert!(stages.contains("`ferrule pr push demo`"), "{stages}");

    let full = ok(
        root,
        &["guide", "stages", "--slug", "demo", "--lane", "full"],
    );
    assert!(
        full.contains("## Full lane\n") && !full.contains("## Fast lane"),
        "{full}"
    );

    let both = ok(root, &["guide", "stages"]);
    assert!(both.contains("## Fast lane (fast lane)\n"), "{both}");
    assert!(both.contains("`ferrule pr push <slug>`"), "{both}");

    let gate4 = ok(root, &["guide", "gate4", "--slug", "demo"]);
    assert!(!gate4.contains("(done)"), "{gate4}");
    seed_log(root, "2026-10-05T09:00", "demo", &["pr=ready"]);
    let gate4 = ok(root, &["guide", "gate4", "--slug", "demo"]);
    assert!(
        gate4.contains("(done)\n- **Approved** → `ferrule gate approve demo gate4`"),
        "{gate4}"
    );
    assert!(!gate4.contains("(done)\n- **CI**"), "{gate4}");

    assert!(!gate4.starts_with("# "), "{gate4}");
    let fast = ok(root, &["guide", "stages", "--section", "fast lane"]);
    assert!(fast.starts_with("## Fast lane"), "{fast}");
    assert!(!fast.contains("## Full lane"), "{fast}");
    let missing = ferrule(root, root, &["guide", "stages", "--section", "Nope"]);
    assert!(
        missing
            .starts_with("guide=unknown-section topic=stages section=Nope\nsection=Every stage\n"),
        "{missing}"
    );
    assert!(missing.ends_with("exit=1"), "{missing}");
}

#[test]
fn it_guide_reads_the_slug_and_stage_of_a_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let wt = root.join(".ferrule/worktrees/demo/src");
    std::fs::create_dir_all(&wt).unwrap();
    let out = ferrule(root, &wt, &["guide", "implementer", "--stage", "s2"]);
    assert!(out.ends_with("exit=0"), "{out}");
    assert!(
        out.contains("`ferrule spec next <PREFIX> --run demo`"),
        "{out}"
    );
    assert!(!out.contains("<slug>"), "{out}");
}

#[test]
fn it_guide_overrides_merge_by_heading() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let guide = root.join(".ferrule/guide");
    std::fs::create_dir_all(&guide).unwrap();
    std::fs::write(
        guide.join("implementer.md"),
        "## Checks\n\n- `cargo fmt --check`\n- `cargo test -p <crate>`\n",
    )
    .unwrap();
    let out = ok(root, &["guide", "implementer"]);
    assert!(
        out.contains("## Checks\n\n- `cargo fmt --check`\n"),
        "{out}"
    );
    assert!(!out.contains("the formatter, in check mode"), "{out}");
    assert!(out.contains("## Stage completion"), "the rest stays: {out}");

    let list = ok(root, &["guide", "--list"]);
    assert!(
        list.contains("implementer     Implementing a stage under TDD (project override)\n"),
        "{list}"
    );
}

#[test]
fn it_guide_check_flags_unknown_topics_and_bad_tags() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert_eq!(ok(root, &["guide", "--check"]), "guide-check=ok\nexit=0");
    let guide = root.join(".ferrule/guide");
    std::fs::create_dir_all(&guide).unwrap();
    std::fs::write(
        guide.join("deploy.md"),
        "# deploy — Shipping a release\n\nThen `ferrule guide show releases`, `ferrule guide show stages --section 'Nope'`.\n\n<!-- lane: slow -->\nx\n",
    )
    .unwrap();
    assert_eq!(
        ferrule(root, root, &["guide", "--check"]),
        "guide-check=fail topic=deploy names-unknown=releases hint=fix the name or add the topic\nguide-check=fail topic=deploy names-unknown=stages#Nope hint=add the heading to the topic or fix the reference\nguide-check=fail topic=deploy bad-tag=<!-- lane: slow --> hint=a tag is <!-- lane: fast|full --> or <!-- done: pr=ready -->\nexit=1"
    );
    assert!(
        ok(root, &["guide", "--list"]).contains("Project:\n  deploy          Shipping a release\n")
    );
    assert_eq!(
        ferrule(root, root, &["guide", "nosuch"]),
        "guide=unknown topic=nosuch list=ferrule guide list\nexit=1"
    );
    assert!(ferrule(root, root, &["guide"]).ends_with("exit=2"));
}
