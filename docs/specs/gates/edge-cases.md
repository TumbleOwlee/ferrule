# Gates — edge cases

## Guide rules

**GA-E-001** — The `fe-review` skill cites the reviewer guide, because its inline review acts as the reviewer.

**GA-E-002** — A project override of a section holding shared text changes that section in the overridden guide only.

**GA-E-003** — A project's existing `.ferrule/guide/decisions.md` is listed as a project topic.

**GA-E-004** — An installed agent file takes no run placeholder and no lane or done tag from shared text.

## Plan gate

**GA-E-005** — `ferrule gate approve <slug> gate2 --live-repo` refuses a value that is not `<owner>/<name>`.

## Stage tiers

**GA-E-006** — A stage of a plan written without tiers gets its own review.

**GA-E-007** — A run resumed mid-batch keeps the batch's committed stages in its review point.

**GA-E-008** — A stage promoted by a spec change closes the open batch, and one review covers the batch's stages and the promoted stage.

**GA-E-009** — On a project whose gauntlet steps name no level, a push and a review point run none of them and gate 3 runs them all.

**GA-E-012** — The stage that lands the brief's spec text does not count as changing the spec, so it keeps its planned tier.

**GA-E-013** — A stage that takes its batch to 1,200 changed lines or more is the last stage its batch's review covers.

**GA-E-014** — A plan whose stage names a risk word that is none of `host-writes`, `persisted-state` and `public-surface` is refused at gate 2 on any tier, naming the stage and the word, and nothing is written.

**GA-E-015** — A draft PR stage line that names no tier rebuilds that stage with the own tier.

## Checks

**GA-E-010** — On a project whose stack is not Rust, a stage's checks run only the push-level steps, not its new tests.

**GA-E-011** — A `{crates}` argument runs as the whole workspace at a push without a stage, at gate 3 and when the diff touches no crate, and is dropped on a project whose stack is not Rust.

**GA-E-019** — A stage whose diff touches no crate runs no review-level steps at its checks.

**GA-E-020** — A stage change that breaks only a dependent crate's tests is caught at the review point or gate 3, not at the stage's checks.

## Stage stops

**GA-E-018** — A stage marked to stop at gate 2 stops after its review even when the user has no decision to make.
