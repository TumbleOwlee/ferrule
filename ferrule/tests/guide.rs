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

/// GA-R-120 — `guide show` has no `--lane` flag.
#[test]
fn it_guide_show_has_no_lane_flag() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let out = ferrule(root, root, &["guide", "stages", "--lane", "full"]);
    assert!(out.ends_with("exit=2"), "{out}");
}

#[test]
fn it_guide_marks_the_cards_done_steps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_card(root, "demo", "open", &["pr=34"]);

    let stages = ok(root, &["guide", "stages", "--slug", "demo"]);
    assert!(!stages.contains("lane"), "{stages}");
    let run_dir = root.join(".ferrule/runs/demo").display().to_string();
    assert!(stages.contains(&format!("`{run_dir}/plan.md`")), "{stages}");
    assert!(stages.contains("`ferrule pr push demo`"), "{stages}");

    let bare = ok(root, &["guide", "stages"]);
    assert!(bare.contains("`ferrule pr push <slug>`"), "{bare}");

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
    let one = ok(root, &["guide", "stages", "--section", "per stage"]);
    assert!(one.starts_with("## Per stage"), "{one}");
    assert!(!one.contains("## Every stage"), "{one}");
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
        "# deploy — Shipping a release\n\nThen `ferrule guide show releases`, `ferrule guide show stages --section 'Nope'`.\n\n<!-- lane: fast -->\nx\n",
    )
    .unwrap();
    assert_eq!(
        ferrule(root, root, &["guide", "--check"]),
        "guide-check=fail topic=deploy names-unknown=releases hint=fix the name or add the topic\nguide-check=fail topic=deploy names-unknown=stages#Nope hint=add the heading to the topic or fix the reference\nguide-check=fail topic=deploy bad-tag=<!-- lane: fast --> hint=a tag is <!-- done: pr=ready -->\nexit=1"
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
    seed_card(root, "demo", "open", &[]);
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
    assert!(
        text.contains(
            "commits and pushes the stage, or its review fixes when it closes a batch, before gate 4, whether or not its stop is shown"
        ),
        "{text}"
    );
}

/// GA-E-018 — a plan-marked stage stops after its review even with nothing to decide.
#[test]
fn it_stage_review_guide_stops_a_plan_marked_stage() {
    says(
        "stage-review",
        &["A plan-marked stage stops after its review even with nothing to decide."],
    );
}

/// RU-R-073 — the live check runs after the last code stage is pushed and before the branch review.
#[test]
fn it_guides_run_the_live_check_before_the_branch_review() {
    says(
        "stage-review",
        &[
            "the implementer commits the last code stage first, `ferrule pr push demo`, the card stays `inreview`; then `ferrule run live-check demo start`, the live check, `ferrule run live-check demo end`; then the branch review",
            "`live-check=refused reason=open-stage` → commit and push the stage it names first.",
        ],
    );
    says(
        "gate3",
        &[
            "With `live-repo` set the live check runs before this review, after the last code stage is committed and pushed",
            "`ferrule run live-check demo start`, the live check, `ferrule run live-check demo end`",
        ],
    );
}

/// RU-R-072, RU-E-035 — gate 1 and resume run the preflight first and relay a refusal.
#[test]
fn it_guides_run_the_preflight_first() {
    says(
        "gate1",
        &[
            "`ferrule run preflight` first: `preflight=refused` → stop, relay its `hint=` to the user, no card is made; then `ferrule run init",
        ],
    );
    says(
        "resume",
        &["Then `ferrule run preflight`: `preflight=refused` → stop and relay its `hint=`."],
    );
}

/// The topic as the full-lane run `demo` reads it.
fn topic(name: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    seed_card(tmp.path(), "demo", "open", &[]);
    ok(tmp.path(), &["guide", "show", name, "--slug", "demo"])
}

fn says(name: &str, phrases: &[&str]) {
    let text = topic(name);
    for p in phrases {
        assert!(text.contains(p), "{name}: missing {p}\n{text}");
    }
}

/// GA-R-069 — the planner's stage table carries a tier and a risk per stage.
#[test]
fn it_planner_guide_proposes_a_tier_per_stage() {
    says(
        "planner",
        &[
            "| Stage | Title | Files | Blocked by | Tier | Risk |",
            "`Tier` is `own`, `batched` or `skipped`",
            "A stage with a risk is never `skipped`; a stage marked `Stop` is `own`",
        ],
    );
}

/// GA-R-075 — a batched stage without a stop is committed and pushed, its card stays inreview.
#[test]
fn it_stages_guide_commits_a_batched_stage_without_a_stop() {
    says(
        "stages",
        &["resume the implementer to commit, `ferrule pr push demo`, the card stays `inreview`"],
    );
}

/// GA-R-088 — a skipped or unbatched stage runs through: commit, push, done, next stage.
#[test]
fn it_stages_guide_runs_a_skipped_stage_through() {
    says(
        "stages",
        &[
            "`review=none` → no review, no stop:",
            "resume the implementer to commit, `ferrule pr push demo`, card → `done`, the next stage",
        ],
    );
}

/// GA-R-104, GA-R-105 — the implementer batches open questions before `inreview`.
#[test]
fn it_implementer_guide_batches_questions_before_review() {
    says(
        "implementer",
        &[
            "Collect your open questions as you work and ask them in one batch before `status=inreview`; ask at once only when you are blocked: the stage cannot go on until the answer comes.",
        ],
    );
}

/// GA-R-075, GA-R-088 — open implementer questions stop once before a batched or skipped commit.
#[test]
fn it_stages_guide_asks_before_a_batched_or_skipped_commit() {
    says(
        "stages",
        &["open implementer questions → one question stop before the commit"],
    );
}

/// GA-R-106, GA-R-110 — first-pass `Needs user` items reach the user before any fix round.
#[test]
fn it_stage_review_guide_relays_needs_user_after_the_first_pass() {
    says(
        "stage-review",
        &[
            "First-pass `Needs user` items → relayed to the user at once, as a question batch, before any fix round; each answer is appended to `decisions.md` as a `(user)` line.",
        ],
    );
}

/// GA-R-107, GA-R-108 — a code-changing answer is committed first and rechecked on its diff only.
#[test]
fn it_stage_review_guide_rechecks_only_a_code_changing_answer() {
    says(
        "stage-review",
        &[
            "An answer that changes code → the implementer commits the reviewed stage first, then applies the answer; the same reviewer's follow-up pass reads only the diff from that commit (`recheck <sha>`).",
            "An answer that changes no code → no review rerun",
        ],
    );
}

/// GA-R-109 — a spec change logged `origin=user` counts as kept.
#[test]
fn it_reviewer_guide_keeps_user_spec_changes() {
    says(
        "reviewer",
        &["A change logged `origin=user` counts as kept: listed, never a `Needs user` item."],
    );
    says(
        "spec-changes",
        &["- **origin=user** → kept: the user asked for it; no `Needs user` item."],
    );
}

/// GA-R-110, GA-R-107 — a settled item is never raised again; a recheck reads only its diff.
#[test]
fn it_reviewer_guide_never_reraises_a_settled_item() {
    says(
        "reviewer",
        &[
            "Never re-raise an item a `(user)` line of `decisions.md` settles. Resumed with `recheck <sha>`: read only `ferrule run diff demo --base <sha>` and `ferrule run diff demo --uncommitted` (the applied answer is uncommitted) and review that change alone.",
        ],
    );
}

/// GA-R-108, GA-R-110 — answers that change no code settle the verdict without a review.
#[test]
fn it_reviewer_guide_settles_answered_items_without_review() {
    says(
        "reviewer",
        &[
            "Resumed with `settle` and the user's answers: re-read nothing, review nothing; rewrite `review.verdict.md` with each answered item moved off `Needs user`",
        ],
    );
    says(
        "stage-review",
        &["resume the same reviewer with the answers to settle the verdict"],
    );
}

/// GA-R-076 — a batch's review fixes are committed once, on the last stage's card.
#[test]
fn it_stage_review_guide_commits_batch_fixes_once() {
    says(
        "stage-review",
        &["A batch's fixes are committed once, when clean, on the last stage's card, and pushed"],
    );
}

/// GA-R-077 — minors are fixed by the stage's implementer before its commit.
#[test]
fn it_minors_are_fixed_before_the_commit() {
    says("verify", &["that stage's implementer fixes every one"]);
    says(
        "implementer",
        &["Minors in your brief are fixed before the commit"],
    );
    says(
        "reviewer",
        &["always `fixed before commit`, by that stage's implementer"],
    );
    says("stage-review", &["no review after the fix"]);
}

/// GA-R-078 — gate 3 reviews the branch whatever the tiers.
#[test]
fn it_gate3_guide_always_reviews_the_branch() {
    says(
        "gate3",
        &["Always a whole-branch review, whatever the stage tiers"],
    );
}

/// GA-R-079 — each pass of the branch review is a fresh reviewer.
#[test]
fn it_gate3_guide_takes_a_fresh_reviewer_per_pass() {
    says("gate3", &["each pass a fresh reviewer"]);
    says("stage-review", &["each pass a fresh reviewer"]);
}

/// GA-R-080 — a batch scope reads every listed stage from the commit before the first.
#[test]
fn it_reviewer_guide_batch_scope_reads_every_stage() {
    says(
        "reviewer",
        &[
            "`batch s<a>,s<b>` (base = the commit before `s<a>`; reads the commits of every listed stage and the uncommitted fixes",
        ],
    );
}

/// GA-R-085 — the implementer reruns no suite the workflow runs.
#[test]
fn it_implementer_guide_reruns_no_workflow_suite() {
    says(
        "implementer",
        &["Run a suite only to see a failure it reported go green, never again before your answer"],
    );
}

/// GA-R-092 — stage and batch review status lines carry the blocker and minor counts.
#[test]
fn it_reviewer_status_line_reports_blockers_and_minors() {
    says(
        "reviewer",
        &["count=<blockers+majors> blockers=<n> user=<Needs user items> minors=<n>"],
    );
}

/// GA-R-094 — the branch scope's status line carries the minor count.
#[test]
fn it_reviewer_branch_status_line_reports_minors() {
    says(
        "reviewer",
        &[
            "`minors=<n>` (the lines under `Minors`, 0 for none) on `stage`, `batch` and `branch` scopes",
        ],
    );
}

/// GA-R-082, GA-R-083, GA-R-084 — the guides call each check level where it applies.
#[test]
fn it_guides_call_each_check_level() {
    says(
        "stages",
        &["`ferrule ci gauntlet demo --level push --stage s<n>`"],
    );
    says(
        "stage-review",
        &["`ferrule ci gauntlet demo --level review`"],
    );
    says("gate3", &["`ferrule ci gauntlet demo` (the full level"]);
}

/// GA-R-115 — the guides say a stage that commits before any review also gets
/// the review-level steps at its checks.
#[test]
fn it_guides_widen_checks_before_an_unreviewed_commit() {
    says(
        "stages",
        &[
            "a batched or skipped stage also the review-level steps over the crates its diff touches",
        ],
    );
    says(
        "implementer",
        &["also clippy and your changed packages' tests when the stage commits before any review"],
    );
    says(
        "verify",
        &["with the review-level steps for a stage that commits before any review"],
    );
}

/// GW-R-036 — an adopted commit takes the stage checks, its tier and its review point.
#[test]
fn it_stages_guide_routes_an_adopted_commit_through_its_tier() {
    says(
        "stages",
        &[
            "The adopted commit then takes the stage checks, `ferrule stage tier` and its review point like any commit",
        ],
    );
}

/// GW-R-014, GW-R-016 — a diverged push refusal goes to the blocked stop with both heads, never to a re-plan.
#[test]
fn it_stages_guide_sends_a_diverged_refusal_to_the_blocked_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "stages", "--slug", "demo"]);
    let line = text
        .lines()
        .find(|l| l.contains("reason=diverged local=<sha> remote=<sha>"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(line.contains("Question and blocked stage"), "{line}");
    assert!(!line.contains("guide show replan"), "{line}");
    let bullet = text
        .lines()
        .find(|l| l.contains("--local <sha> --remote <sha>"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        bullet.contains("ferrule prompt render demo blocked"),
        "{bullet}"
    );
    assert!(!bullet.contains("guide show replan"), "{bullet}");
    assert!(bullet.contains("rebase through ferrule"), "{bullet}");
    assert!(bullet.contains("resolve it outside ferrule"), "{bullet}");
    assert!(bullet.contains("never force-push by hand"), "{bullet}");
}

/// GW-R-016 — the blocked stop's `1: a` rebases, `1: b` pushes again, `1: c` halts the run.
#[test]
fn it_stages_guide_rebases_or_pushes_again_after_a_divergence() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "stages", "--slug", "demo"]);
    let bullet = text
        .lines()
        .find(|l| l.contains("--local <sha> --remote <sha>"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        bullet.contains("`1: a` → `ferrule run rebase demo`"),
        "{bullet}"
    );
    assert!(
        bullet.contains("`1: b` → `ferrule pr push demo` again"),
        "{bullet}"
    );
    assert!(bullet.contains("`1: c` → the run halts"), "{bullet}");
}

/// GW-R-082 — a sha push refusal goes to the sha stop with the refusal's `mismatch=`, never to a re-plan.
#[test]
fn it_stages_guide_sends_a_sha_refusal_to_the_sha_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let text = ok(root, &["guide", "show", "stages", "--slug", "demo"]);
    let line = text
        .lines()
        .find(|l| l.contains("reason=sha mismatch=<why>"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(line.contains("Question and blocked stage"), "{line}");
    assert!(line.contains("never a re-plan"), "{line}");
    assert!(!line.contains("guide show replan"), "{line}");
    let bullet = text
        .lines()
        .find(|l| l.contains("--mismatch <why>") && l.starts_with("- A sha push refusal"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        bullet.contains("ferrule prompt render demo blocked"),
        "{bullet}"
    );
    assert!(!bullet.contains("guide show replan"), "{bullet}");
}

/// GW-R-090, GW-R-091, S4 — the sha stop's `1: a` continues with the first uncommitted stage and pushes after its commit, `1: b` pushes again, `1: c` halts the run.
#[test]
fn it_stages_guide_continues_or_pushes_again_after_a_sha_refusal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &["lane=full"]);
    let text = ok(root, &["guide", "show", "stages", "--slug", "demo"]);
    let bullet = text
        .lines()
        .find(|l| l.contains("--mismatch <why>") && l.starts_with("- A sha push refusal"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(bullet.contains("`1: a` →"), "{bullet}");
    assert!(
        bullet.contains("first stage in plan order whose card has no `commit=`"),
        "{bullet}"
    );
    assert!(
        bullet.contains("`ferrule pr push demo` after its commit"),
        "{bullet}"
    );
    assert!(
        bullet.contains("`1: b` → `ferrule pr push demo` again"),
        "{bullet}"
    );
    assert!(
        bullet.contains("the same refusal returning to the same stop"),
        "{bullet}"
    );
    assert!(bullet.contains("`1: c` → the run halts"), "{bullet}");
}

/// GW-R-058 — a rebase conflict goes to the implementer of the newest committed stage.
#[test]
fn it_stages_guide_sends_a_rebase_conflict_to_the_implementer() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "stages", "--slug", "demo"]);
    let bullet = text
        .lines()
        .find(|l| l.contains("`rebase=conflict paths=<n>`"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(bullet.contains("newest"), "{bullet}");
    assert!(bullet.contains("commit="), "{bullet}");
    assert!(bullet.contains("status=rebased"), "{bullet}");
}

/// GW-R-060 — `status=rebased` reruns the rebase command, then pushes.
#[test]
fn it_stages_guide_reruns_the_rebase_after_status_rebased() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "stages", "--slug", "demo"]);
    let bullet = text
        .lines()
        .find(|l| l.contains("`status=rebased` →"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        bullet.contains("`ferrule run rebase demo` again"),
        "{bullet}"
    );
    assert!(bullet.contains("`ferrule pr push demo`"), "{bullet}");
    assert!(bullet.contains("rebase=refused"), "{bullet}");
}

/// GW-R-059 — the implementer resolves only the conflicted files and ends on `status=rebased`.
#[test]
fn it_implementer_guide_resolves_only_the_conflicted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let section = ok(
        root,
        &[
            "guide",
            "show",
            "implementer",
            "--slug",
            "demo",
            "--section",
            "Rebase conflict",
        ],
    );
    assert!(section.contains("--diff-filter=U"), "{section}");
    assert!(
        section.contains("GIT_EDITOR=true git rebase --continue"),
        "{section}"
    );
    assert!(section.contains("status=rebased"), "{section}");
}

/// GW-R-052 — a run whose PR conflicts with its base is rebased through ferrule.
#[test]
fn it_resume_guide_rebases_a_pr_conflicting_with_its_base() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "resume", "--slug", "demo"]);
    let line = text
        .lines()
        .find(|l| l.contains("conflicts with its base"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(line.contains("`ferrule run rebase demo`"), "{line}");
    assert!(line.contains("`ferrule pr push demo`"), "{line}");
}

/// GW-R-020 — resume names a diverged row and resolves it by rebase or outside ferrule.
#[test]
fn it_resume_guide_resolves_a_diverged_row_outside_ferrule() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "resume", "--slug", "demo"]);
    let row = text
        .lines()
        .find(|l| l.contains("`diverged <b> local=<sha> remote=<sha>`"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(row.contains("outside ferrule"), "{row}");
    assert!(row.contains("`ferrule run rebase demo`"), "{row}");
    assert!(row.contains("never by a force-push by hand"), "{row}");
}

/// GA-R-008, GA-R-009, GA-R-010 — the branch review reads an uncommitted last stage and its spec changes.
#[test]
fn it_reviewer_guide_reads_the_uncommitted_stage() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
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
    seed_card(root, "demo", "open", &[]);
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
            "Scopes `stage s<n>`, `batch` and `branch` also read `ferrule run diff demo --uncommitted --specs`"
        ),
        "{specs}"
    );
}

/// GA-R-014 — the review's read of the spec rules succeeds on the shipped README.
#[test]
fn it_reviewer_guide_reads_only_sections_the_shipped_readme_has() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
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

/// GA-R-015, GA-R-017, GA-R-018, GA-R-019, GA-R-064 — each guide states the rules its agent works or reviews by.
#[test]
fn it_guides_state_the_tdd_and_standards_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let tdd = ok(
        root,
        &["guide", "implementer", "--slug", "demo", "--section", "TDD"],
    );
    assert!(tdd.contains("1. Write the stage's tests first; once each compiles (a stub of the code under test where needed), record its failure with `ferrule stage red demo <test>`, which runs it and logs each failure it sees."), "{tdd}");
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
    assert!(tdd.contains("4. A test that passes from the start pins existing behavior: mark it with `ferrule stage pins demo <test> -- <what it pins>`."), "{tdd}");
    assert!(tdd.contains("On a Rust project `ferrule stage commit` refuses a new test without a proof (`reason=test-unproven`) and a removed or rewritten test without `--replaced <old>=<new>` (`reason=test-unreplaced`); a rewritten test may name itself and then needs a proof."), "{tdd}");
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
    assert!(!axes.contains("TDD honesty"), "{axes}");
    assert!(!axes.contains("test-red"), "{axes}");
    for topic in ["reviewer", "reviewer-plan"] {
        let whole = ok(root, &["guide", topic, "--slug", "demo"]);
        assert!(!whole.contains("TDD honesty"), "{topic}: {whole}");
    }
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
    seed_card(root, "demo", "open", &[]);
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
        text.contains("The Standards axis is checked against the rules it states."),
        "{text}"
    );
    assert!(!text.contains("rules on the spec"), "{text}");
}

/// GA-R-023, GA-R-024, GA-R-025, GA-R-027, GA-R-028 — each guide states what its own reader needs.
#[test]
fn it_guides_state_what_their_reader_needs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
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
        implementer.contains("A stage's `files` list is advisory: a file outside it is noted on the card by `ferrule stage commit`."),
        "{implementer}"
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

/// GA-R-031 — every agent that asks states the question batch itself, in the same text.
#[test]
fn it_asking_agents_state_the_question_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let section = |topic| {
        ok(
            root,
            &["guide", topic, "--slug", "demo", "--section", "Questions"],
        )
    };
    let body = |text: &str| {
        text.lines()
            .filter(|line| !line.starts_with("Title line of ") && !line.starts_with("exit="))
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    };
    let author = section("author");
    for needle in [
        "Collect every decision you can state without knowing another's answer into one file.",
        "One block per decision:",
        "## 1. <decision, one line, as a question>",
        "`Why you:` is mandatory.",
    ] {
        assert!(author.contains(needle), "{author}");
    }
    for (topic, title) in [
        ("architect", "architecture"),
        ("planner", "gate 2"),
        ("implementer", "s<n>"),
    ] {
        let text = section(topic);
        assert!(
            text.contains(&format!("# demo — questions ({title})")),
            "{text}"
        );
        for needle in [
            "Collect every decision you can state without knowing another's answer into one file.",
            "One block per decision:",
            "`Why you:` is mandatory.",
        ] {
            assert!(text.contains(needle), "{topic}: {needle}");
        }
        assert_eq!(body(&text), body(&author), "{topic}");
    }
}

/// GA-R-032 — the plan review states the title rules it checks.
#[test]
fn it_plan_review_states_the_stage_title_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "reviewer-plan", "--slug", "demo"]);
    assert!(text.contains("imperative, verb first; ≤ 3 words, ≤ 20 characters; letters, digits, hyphens, spaces only; unique within the plan; no stage id"), "{text}");
    assert!(
        text.contains("`s0`, when present, is always `Land spec`"),
        "{text}"
    );
}

/// GA-R-033 — the follow-up pass names a topic a reader can open.
#[test]
fn it_reviewer_follow_up_names_the_plan_checks_topic() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(
        root,
        &[
            "guide",
            "reviewer",
            "--slug",
            "demo",
            "--section",
            "Follow-up pass",
        ],
    );
    assert!(
        text.contains("every check of `ferrule guide show reviewer-plan`"),
        "{text}"
    );
    assert!(!text.contains("fe-reviewer-plan.md"), "{text}");
}

/// GA-R-034, GA-R-035 — the implementer and its review get the same ID citation rule.
#[test]
fn it_implementer_and_reviewer_state_the_id_citation_rule() {
    const RULE: &str = "test's doc comment lists every requirement or edge-case ID its assertions directly verify, and only those, before the summary (`/// AB-R-012, AB-R-157 — …`)";
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let tdd = ok(
        root,
        &["guide", "implementer", "--slug", "demo", "--section", "TDD"],
    );
    assert!(tdd.contains(RULE), "{tdd}");
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
    assert!(axes.contains(RULE), "{axes}");
    assert!(!axes.contains("ID citation placement"), "{axes}");
}

/// GA-R-036 — the implementer reads the rules that bind a spec change.
#[test]
fn it_implementer_reads_the_spec_rules_before_a_spec_change() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(
        root,
        &[
            "guide",
            "implementer",
            "--slug",
            "demo",
            "--section",
            "Spec changes",
        ],
    );
    assert!(text.contains("Before a spec change, read the rules that bind the text: `ferrule file section 'Rules for writing specs' docs/specs/README.md`"), "{text}");
}

/// GA-R-060, GA-R-061 — the planner and re-plan guides read the rules that bind a spec edit first.
#[test]
fn it_planner_guides_read_the_spec_rules_before_a_spec_edit() {
    const READ: &str = "Before you edit an entry, read the rules that bind the text: `ferrule file section 'Rules for writing specs' docs/specs/README.md`.";
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    for guide in ["planner", "planner-replan"] {
        let text = ok(
            root,
            &[
                "guide",
                guide,
                "--slug",
                "demo",
                "--section",
                "Interview before drafting",
            ],
        );
        let read = text.find(READ).unwrap_or_else(|| panic!("{guide}: {text}"));
        let edit = text
            .find("edit `spec-diff.md` in place")
            .unwrap_or_else(|| panic!("{guide}: {text}"));
        assert!(read < edit, "{guide}: {text}");
    }
}

/// GA-R-037 — the bootstrap suggests only the override that keeps the guide's text.
#[test]
fn it_bootstrap_suggests_no_project_rules_override() {
    let skill = include_str!("../../ferrule-core/install/claude/skills/fe-bootstrap.md");
    assert!(!skill.contains("## Project rules"), "{skill}");
    assert!(skill.contains("with a `## Checks` section"), "{skill}");
}

/// GA-R-038 — the re-plan guide states the question format itself.
#[test]
fn it_replan_guide_states_the_question_format() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "planner-replan", "--slug", "demo"]);
    for needle in [
        "# demo — questions (gate 2)",
        "One block per decision:",
        "`Why you:` is mandatory.",
    ] {
        assert!(text.contains(needle), "{needle}: {text}");
    }
    assert!(!text.contains("ferrule guide show planner"), "{text}");
}

/// GW-R-006, S2 — the implementer guide blocks a clean-worktree nothing-staged refusal with `head=` and an `--adopt` suggestion.
#[test]
fn it_implementer_guide_blocks_a_clean_nothing_staged() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(
        root,
        &[
            "guide",
            "implementer",
            "--slug",
            "demo",
            "--section",
            "Ask or stop — never improvise",
        ],
    );
    assert!(
        text.contains("`commit=refused reason=nothing-staged` on a clean worktree"),
        "{text}"
    );
    assert!(text.contains("`head=<sha>` in the reason"), "{text}");
    assert!(text.contains("--adopt <sha>"), "{text}");
    assert!(!text.contains("reason=files"), "{text}");
    assert!(text.contains("`reason=merge`"), "{text}");
    assert!(text.contains("`reason=unlogged`"), "{text}");
    assert!(text.contains("`reason=no-merge-base`"), "{text}");
}

/// GW-R-038, S2 — a blocked reason that suggests `--adopt` takes the blocked stop, and `1: a` adopts.
#[test]
fn it_stages_guide_routes_an_adopt_suggestion_to_the_blocked_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let section = ok(
        root,
        &[
            "guide",
            "stages",
            "--slug",
            "demo",
            "--section",
            "Question and blocked stage",
        ],
    );
    let line = section
        .lines()
        .find(|l| l.contains("--adopt"))
        .unwrap_or_else(|| panic!("{section}"));
    assert!(line.contains("1: a"), "{line}");
    assert!(!line.contains("guide show replan"), "{line}");
}

/// RU-R-008, RU-R-009, RU-R-010 — the cleanup guide states the CI exemptions
/// beside the epic's, the issue refusals and the closing notices.
#[test]
fn it_cleanup_guide_states_exemptions_and_notices() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "cleanup", "--slug", "demo"]);
    for want in [
        "Done · demo · issue #<n> · cleaned up · archived",
        "Done · demo · cleaned up · archived",
        "issue-waived",
        "request run",
        "an epic",
        "without a PR whose issue is not open",
    ] {
        assert!(text.contains(want), "{want}: {text}");
    }
}

/// RU-R-066, RU-R-069 — the cleanup guide states the waived notice and that a
/// `ci-waived` line lets through a PR run with no `ci=` line.
#[test]
fn it_cleanup_guide_states_the_ci_waiver() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "cleanup", "--slug", "demo"]);
    for want in [
        "Done · demo · PR #<pr> ready · CI waived · cleaned up · archived",
        "no-ci",
        "a ci-waived line also lets through a PR run with no ci= line",
    ] {
        assert!(text.contains(want), "{want}: {text}");
    }
}

/// S4, RU-R-068 — the gate 4 guide has the workflow tell the user when checks
/// never arrive and log `ci-waived` only on the user's word.
#[test]
fn it_gate4_guide_waives_missing_checks_on_the_users_word() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "gate4", "--slug", "demo"]);
    let line = text
        .lines()
        .find(|l| l.contains("reason=no-checks"))
        .unwrap_or_else(|| panic!("no no-checks line: {text}"));
    for want in [
        "after retries",
        "tell the user",
        "on the user's word only",
        "ferrule board log demo ci-waived -- <why>",
    ] {
        assert!(line.contains(want), "{want}: {line}");
    }
}

/// RU-R-056, RU-R-057, RU-R-058 — the cleanup guide names abandon, its
/// confirmation on the user's word and its two notices.
#[test]
fn it_cleanup_guide_names_abandon_and_its_notices() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "cleanup", "--slug", "demo"]);
    for want in [
        "ferrule run abandon demo",
        "ferrule run abandon demo --confirm",
        "only on the user's word",
        "Abandoned · demo · issue #<n> open · archived",
        "Abandoned · demo · archived",
    ] {
        assert!(text.contains(want), "{want}: {text}");
    }
}

/// The principles guide names both endings of a run: a ready PR
/// with green CI, or cleanup of a run without a PR under its CI exemption.
#[test]
fn it_principles_guide_names_the_run_without_a_pr_ending() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let principles = ok(root, &["guide", "principles", "--slug", "demo"]);
    assert!(
        principles.contains("a run without a PR ends at `ferrule run cleanup`"),
        "{principles}"
    );
}

/// GA-R-044, GA-R-111 — each guide that records decisions states the line shape and its rules.
#[test]
fn it_guides_state_the_decision_line_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    for topic in ["author", "architect", "planner", "implementer"] {
        let text = ok(
            root,
            &[
                "guide",
                topic,
                "--slug",
                "demo",
                "--section",
                "Decision lines",
            ],
        );
        for needle in [
            "- <choice> (<user | chosen>) [",
            "Append only",
            "A reversal line's why starts `supersedes \"<earlier choice>\"`",
        ] {
            assert!(text.contains(needle), "{topic}: {text}");
        }
        assert!(!text.contains("{run_dir}"), "{topic}: {text}");
    }
}

/// GA-R-048, GA-R-049 — the `decisions` topic no longer ships.
#[test]
fn it_guide_decisions_topic_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let out = ferrule(root, root, &["guide", "show", "decisions"]);
    assert!(!out.ends_with("exit=0"), "{out}");
    assert!(
        out.contains("guide=refused reason=unknown topic=decisions"),
        "{out}"
    );
    let list = ok(root, &["guide", "list"]);
    assert!(
        !list
            .lines()
            .any(|l| l.split_whitespace().next() == Some("decisions")),
        "{list}"
    );
}

/// GA-E-002 — an override of a section holding shared text changes that guide only.
#[test]
fn it_guide_override_of_shared_text_stays_in_that_guide() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let guide = root.join(".ferrule/guide");
    std::fs::create_dir_all(&guide).unwrap();
    std::fs::write(guide.join("planner.md"), "## Decision lines\n\nmine\n").unwrap();
    let planner = ok(root, &["guide", "planner", "--slug", "demo"]);
    assert!(
        planner.contains("mine") && !planner.contains("Append only"),
        "{planner}"
    );
    let author = ok(root, &["guide", "author", "--slug", "demo"]);
    assert!(author.contains("Append only"), "{author}");
}

/// GA-E-003 — a project's own `decisions` file is a project topic.
#[test]
fn it_project_decisions_file_is_a_project_topic() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let guide = root.join(".ferrule/guide");
    std::fs::create_dir_all(&guide).unwrap();
    std::fs::write(guide.join("decisions.md"), "# decisions — ours\n\nbody\n").unwrap();
    let list = ok(root, &["guide", "list"]);
    let (_, project) = list.split_once("Project:").expect("a Project heading");
    assert!(project.contains("decisions"), "{list}");
    let shown = ok(root, &["guide", "show", "decisions"]);
    assert!(shown.contains("body"), "{shown}");
}

/// GA-R-045 — a job guide cites no other guide.
#[test]
fn it_job_guides_cite_no_other_guide() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    for topic in [
        "author-late",
        "architect-epic",
        "planner-replan",
        "reviewer-plan",
    ] {
        let text = ok(root, &["guide", topic, "--slug", "demo"]);
        assert!(!text.contains("ferrule guide show "), "{topic}: {text}");
    }
}

/// GA-R-046, GA-R-052, GA-R-053, GA-R-054 — a job guide states what its job needs, read alone.
#[test]
fn it_job_guides_state_what_their_job_needs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let shape = "- <choice> (<user | chosen>) [";
    let mut missing = Vec::new();
    for (topic, needles) in [
        (
            "author-late",
            &[
                "**Concise, compact, facts only.**",
                "Orchestrator relays between you and the user",
                "`AGENTS.md` is already in your context",
                "the brief carries the reason and the entry it narrows",
            ][..],
        ),
        (
            "architect-epic",
            &[
                "Lay down the structure the change is built on",
                "Look up facts yourself",
                shape,
                "# demo — architecture (rev N)",
                "Every decision of the run",
            ][..],
        ),
        (
            "planner-replan",
            &[
                "Draft the implementation plan",
                "Look up facts yourself",
                shape,
                "## Stage title",
                "| Stage | Title | Files | Blocked by |",
                "Never create/move task cards",
                "Every decision of the run",
            ][..],
        ),
        (
            "reviewer-plan",
            &[
                "`AGENTS.md` is already in your context",
                "**`plan.md`:**",
                "**`decisions.md`**",
                "Resumed in the same scope",
                "Never approve a fix because it matches",
                "Verdict: <clean — ready for approval",
            ][..],
        ),
    ] {
        let text = ok(root, &["guide", topic, "--slug", "demo"]);
        for needle in needles {
            if !text.contains(needle) {
                missing.push(format!("{topic}: {needle}"));
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

/// S8 — a tripped size guard hands the architect to the epic guide.
#[test]
fn it_architect_guide_hands_a_tripped_size_guard_to_the_epic_guide() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(
        root,
        &[
            "guide",
            "architect",
            "--slug",
            "demo",
            "--section",
            "Epics, slices, size guard",
        ],
    );
    assert!(text.contains("ferrule guide show architect-epic"), "{text}");
    assert!(text.contains("replaces this guide"), "{text}");
}

/// RU-R-031 — the epic guide states that a GitHub epic is a milestone and
/// that slices are the issues in it.
#[test]
fn it_epic_guide_states_a_github_epic_is_a_milestone() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "epic", "--slug", "demo"]);
    for want in [
        "a GitHub epic is a milestone",
        "slices are the issues in it",
        "milestone-change.md",
    ] {
        assert!(text.contains(want), "{want}: {text}");
    }
}

/// RU-R-035 — the epic guide's closing notice names the milestone.
#[test]
fn it_epic_guide_closing_notice_names_the_milestone() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = ok(root, &["guide", "show", "epic", "--slug", "demo"]);
    assert!(
        text.contains(
            "Epic · demo · milestone #<m> · slices #a, #b, … filed · start with /fe-feature on #a"
        ),
        "{text}"
    );
}

fn rendered(root: &Path, topic: &str) -> String {
    ok(root, &["guide", topic, "--slug", "demo"])
}

/// GA-R-063 — the implementer checks that each cited spec ID is asserted before answering.
#[test]
fn it_implementer_guide_checks_cited_ids_before_review() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = rendered(root, "implementer");
    assert!(
        text.contains(
            "check that each spec ID a test's doc comment cites is asserted by that test"
        ),
        "{text}"
    );
}

/// GA-R-065 — the reviewer is pointed at the card's notes, which never block.
#[test]
fn it_reviewer_guide_hands_over_the_card_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = rendered(root, "reviewer");
    assert!(text.contains("The stage card's notes (`ferrule board show demo.s<n>`) — `files-outside=`, `test-pins` and `test-replaced` lines — point at changes worth a look"), "{text}");
    assert!(
        text.contains("the note itself never blocks and never reaches a user stop"),
        "{text}"
    );
    assert!(!text.contains("is a blocker**."), "{text}");
}

/// GA-R-066 — a plan bookkeeping gap is noted on the card, never a stop.
#[test]
fn it_implementer_guide_logs_bookkeeping_gaps() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = rendered(root, "implementer");
    assert!(
        text.contains("is no stop: log `note -- bookkeeping: <gap>` on the card and go on"),
        "{text}"
    );
    assert!(!text.contains("a file outside your set"), "{text}");
    assert!(
        text.contains("`reason=test-unproven` and `reason=test-unreplaced` are fixed per their hint, not a stop"),
        "{text}"
    );
}

/// GA-R-067 — a plan bookkeeping gap is no review finding.
#[test]
fn it_reviewer_guide_rates_no_finding_for_bookkeeping() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let text = rendered(root, "reviewer");
    assert!(
        text.contains("is a finding when it is scope creep"),
        "{text}"
    );
    assert!(text.contains("is no finding"), "{text}");
    let plan = rendered(root, "reviewer-plan");
    assert!(!plan.contains("covers what its steps touch"), "{plan}");
}

/// GA-R-068 — only what `## Public surface` lists binds the plan and the code.
#[test]
fn it_guides_bind_only_the_public_surface_of_sketches() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    seed_card(root, "demo", "open", &[]);
    let reviewer = rendered(root, "reviewer");
    assert!(
        reviewer.contains("A sketch in `## Interfaces` binds only what `## Public surface` lists"),
        "{reviewer}"
    );
    assert!(
        !reviewer.contains("a method added, dropped or re-typed"),
        "{reviewer}"
    );
    assert!(
        !reviewer.contains("A signature that differs from `## Interfaces`"),
        "{reviewer}"
    );
    let plan = rendered(root, "reviewer-plan");
    assert!(plan.contains("every signature of what `## Public surface` lists appears as written in the step that builds it"), "{plan}");
    let planner = rendered(root, "planner");
    assert!(
        planner.contains("only what its `## Public surface` lists binds"),
        "{planner}"
    );
    assert!(
        !planner.contains("a signature in `## Interfaces` changed,"),
        "{planner}"
    );
    let architect = rendered(root, "architect");
    assert!(architect.contains("Only what `## Public surface` lists binds the plan and the code; the rest of a sketch guides them"), "{architect}");
}
