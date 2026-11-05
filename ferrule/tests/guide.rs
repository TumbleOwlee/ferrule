//! `ferrule guide`: topics cut to the run's card, project overrides, the
//! soundness check.

mod golden;

use std::path::Path;

use golden::{Run, seed_card, seed_log};

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
    seed_card(root, "demo", "open", &["lane=fast", "pr=34"]);

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
            .starts_with("guide=refused reason=unknown-section topic=stages section=Nope hint=pick a heading from the section= lines that follow\nsection=Every stage\n"),
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
        "guide=refused reason=unknown topic=nosuch hint=`ferrule guide list` names the topics\nexit=1"
    );
    assert!(ferrule(root, root, &["guide"]).ends_with("exit=2"));
}

/// GA-R-004 — the last stage's approved stop commits and pushes before gate 4.
#[test]
fn it_stage_review_guide_pushes_before_gate4() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let text = ok(
        root,
        &[
            "guide",
            "stage-review",
            "--slug",
            "demo",
            "--section",
            "Stage stop",
        ],
    );
    assert!(
        text.contains(
            "approved → `ferrule gate approve demo stage`, commit, `ferrule pr push demo`, gate 4"
        ),
        "{text}"
    );
}

/// GA-R-008, GA-R-009, GA-R-010 — the branch review reads an uncommitted last stage and its spec changes.
#[test]
fn it_reviewer_guide_reads_the_uncommitted_stage() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let section = |name| {
        ok(
            root,
            &["guide", "reviewer", "--slug", "demo", "--section", name],
        )
    };
    let scope = section("Scope");
    assert!(
        scope.contains(
            "`ferrule run diff demo --base <base>` and `ferrule run diff demo --uncommitted`"
        ),
        "{scope}"
    );
    assert!(scope.contains("read every `??` path whole"), "{scope}");
    let specs = section("Spec changes");
    assert!(
        specs.contains("`ferrule run diff demo --uncommitted --specs`"),
        "{specs}"
    );
    assert!(specs.contains("a `??` path under `docs/specs/`"), "{specs}");
}

/// GA-R-011, GA-R-012, GA-R-013 — the per-stage review reads its uncommitted stage and its spec changes.
#[test]
fn it_reviewer_guide_reads_the_uncommitted_stage_in_stage_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let section = |name| {
        ok(
            root,
            &["guide", "reviewer", "--slug", "demo", "--section", name],
        )
    };
    let scope = section("Scope");
    let line = scope
        .lines()
        .find(|l| l.contains("**Scope `stage s<n>` or `branch`:**"))
        .expect("the stage scope's paragraph");
    assert!(
        line.contains(
            "`ferrule run diff demo --base <base>` and `ferrule run diff demo --uncommitted`"
        ),
        "{line}"
    );
    assert!(line.contains("read every `??` path whole"), "{line}");
    let specs = section("Spec changes");
    assert!(
        specs.contains(
            "Scopes `stage s<n>` and `branch` also read `ferrule run diff demo --uncommitted --specs`"
        ),
        "{specs}"
    );
}

/// GA-R-014 — the review's read of the spec rules succeeds on the shipped README.
#[test]
fn it_reviewer_guide_reads_only_sections_the_shipped_readme_has() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    std::fs::create_dir_all(root.join("docs/specs")).unwrap();
    std::fs::write(
        root.join("docs/specs/README.md"),
        include_str!("../../ferrule-core/install/specs-readme.md"),
    )
    .unwrap();
    let rules = ok(
        root,
        &[
            "guide",
            "reviewer",
            "--slug",
            "demo",
            "--section",
            "Project rules",
        ],
    );
    assert!(
        rules.contains("`ferrule file section 'Rules for writing specs' docs/specs/README.md`"),
        "{rules}"
    );
    assert!(
        !rules.contains("Requirements intentionally not unit-tested"),
        "{rules}"
    );
    let read = ok(
        root,
        &[
            "file",
            "section",
            "Rules for writing specs",
            "docs/specs/README.md",
        ],
    );
    assert!(read.contains("**Requirements are testable.**"), "{read}");
}

/// GA-R-015, GA-R-016, GA-R-017, GA-R-018, GA-R-019 — each guide states the rules its agent works or reviews by.
#[test]
fn it_guides_state_the_tdd_and_standards_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let tdd = ok(
        root,
        &["guide", "implementer", "--slug", "demo", "--section", "TDD"],
    );
    assert!(tdd.contains("1. Write the stage's tests first and see each fail; log the failure seen with `test-red`."), "{tdd}");
    assert!(
        tdd.contains("2. Write the code until they pass, then log `green`."),
        "{tdd}"
    );
    assert!(
        tdd.contains(
            "3. Never change a test or a spec entry to pass when the code is what is wrong."
        ),
        "{tdd}"
    );
    assert!(tdd.contains("4. A test that passes from the start pins existing behavior: say so with a `note` on your card."), "{tdd}");
    let axes = ok(
        root,
        &[
            "guide",
            "reviewer",
            "--slug",
            "demo",
            "--section",
            "Axes, reported separately",
        ],
    );
    assert!(
        axes.contains("a `test-red` line naming the test and the failure, before `green`"),
        "{axes}"
    );
    assert!(
        axes.contains("the code came after, until the tests passed"),
        "{axes}"
    );
    assert!(
        axes.contains("no test or spec entry changed to pass when the code was what was wrong"),
        "{axes}"
    );
    assert!(
        axes.contains("a test passing from the start pins existing behavior, and a `note` says so"),
        "{axes}"
    );
    assert!(!axes.contains("## TDD"), "{axes}");
    assert!(!axes.contains("ferrule guide show implementer"), "{axes}");
    assert!(
        axes.contains("every rule `AGENTS.md` states, under any heading"),
        "{axes}"
    );
    assert!(!axes.contains("## Conventions — code"), "{axes}");
    assert!(
        axes.contains("a comment rule the project states sets its own severity, otherwise a minor"),
        "{axes}"
    );
}

/// GA-R-020 — the agent guides name no category of rule.
#[test]
fn it_agent_guides_take_the_rules_agents_md_states() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let rules = |topic| {
        ok(
            root,
            &[
                "guide",
                topic,
                "--slug",
                "demo",
                "--section",
                "Project rules",
            ],
        )
    };
    for topic in ["author", "architect", "planner", "implementer"] {
        let text = rules(topic);
        assert!(
            text.contains("The rules it states bind you here."),
            "{text}"
        );
        assert!(!text.contains("rules on the spec"), "{text}");
    }
    let text = rules("reviewer");
    assert!(
        text.contains("The Standards axis is checked against the rules it states;"),
        "{text}"
    );
    assert!(!text.contains("rules on the spec"), "{text}");
}

/// GA-R-023, GA-R-024, GA-R-025, GA-R-026, GA-R-027, GA-R-028 — each guide states what its own reader needs.
#[test]
fn it_guides_state_what_their_reader_needs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let guide = |topic| ok(root, &["guide", topic, "--slug", "demo"]);
    let principles = guide("principles");
    assert!(principles.contains("**A non-behavior change, a change that needs no observable behavior to change, skips gate 1**"), "{principles}");
    for topic in ["author", "architect", "planner"] {
        let text = guide(topic);
        assert!(
            text.contains("`R<n>` for a requirement, `E<n>` for an edge case, each series from 1"),
            "{topic}"
        );
    }
    let implementer = guide("implementer");
    assert!(
        implementer.contains("changing only the files their `files` lists name"),
        "{implementer}"
    );
    let reviewer = guide("reviewer");
    assert!(
        reviewer.contains(
            "**Any file in the diff outside the in-scope stages' `files` lists is a blocker**."
        ),
        "{reviewer}"
    );
    let author = guide("author");
    assert!(
        author.contains("`ferrule issue view <n>`: the body, then one line per comment"),
        "{author}"
    );
    let planner = guide("planner");
    assert!(
        planner.contains("split so each entry holds one rule"),
        "{planner}"
    );
}
