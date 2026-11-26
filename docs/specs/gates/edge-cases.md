# Gates — edge cases

## Guide rules

**GA-E-001** — The `fe-review` skill cites the reviewer guide, because its inline review acts as the reviewer.

**GA-E-002** — A project override of a section holding shared text changes that section in the overridden guide only.

**GA-E-003** — A project's existing `.ferrule/guide/decisions.md` is listed as a project topic.

**GA-E-004** — An installed agent file takes no run placeholder and no done tag from shared text.

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

**GA-E-010** — A stage's checks run no new tests of a test command that declares no test detection; the push-level steps still run.

**GA-E-011** — A `{crates}` argument runs as the whole workspace at a push without a stage, at gate 3 and when the diff touches no crate, and is dropped on a project whose stack is not Rust.

**GA-E-019** — A stage whose diff touches no crate runs no review-level steps at its checks.

**GA-E-020** — A stage change that breaks only a dependent crate's tests is caught at the review point or gate 3, not at the stage's checks.

**GA-E-040** — A test the stage checks run that exceeds the test command's timeout ends the checks with `gauntlet=fail step=test:<name> reason=timeout`.

## Review rules

**GA-E-016** — A clean review reporting `minors=0` routes straight to its stop, with no fix step.

**GA-E-017** — A stop or auto-continue notice states no minors fixed when a stop was already logged after the newest review.

**GA-E-026** — A plan or re-plan review reporting `findings` with `count=0` ends the review with no revise round.

## Stage stops

**GA-E-018** — A stage marked to stop at gate 2 stops after its review even when the user has no decision to make.

## Early stops

**GA-E-021** — A `lane` value already on a saved card is ignored.

**GA-E-022** — A `gate3` date already on a saved card is ignored.

**GA-E-023** — `ferrule gate approve <slug> delta` is a usage error (exit 2).

**GA-E-024** — An `approved=delta` or `stop kind=approval gate=delta` line already on a saved card imports and reads without error.

**GA-E-025** — `ferrule prompt render <slug> delta` is a usage error (exit 2).

## Design checks

**GA-E-027** — On a Claude Code release older than 2.1.271 the comprehension check runs with the project's instruction files in its context, since such a release ignores the setting that leaves them out.

**GA-E-028** — In a run without `spec-diff.md` (a non-behavior change), the comprehension check reads `architecture.md` alone.

**GA-E-029** — A check result written for other revisions of `spec-diff.md` or `architecture.md` than the current ones counts as no result.

**GA-E-030** — `changed` counts only findings named in a `change` answer, so a finding that changed the user's thinking without one is not counted.

**GA-E-031** — A `change` answer that names no finding logs `changed=0` for both checks.

**GA-E-032** — A stop shown again before it was answered (a held run resumed) logs `design-check` events only for its last showing.

**GA-E-033** — A finding ID named in a `change` answer that the stop did not show is not counted in `changed`.

**GA-E-034** — A finding ID named in a `change` answer counts whatever its letter case: `c2` names C2.

**GA-E-035** — A finding ID counts only as a whole word of the `change` answer: `C2,` and `(C2)` name C2, `C20` and `C2a` do not.

**GA-E-036** — A finding named more than once in one `change` answer counts once in `changed`.

**GA-E-037** — `ferrule gate change` on a run whose `arch` field already holds a value is refused and logs no `design-check` event.

**GA-E-038** — A repeated `ferrule gate approve` of an architecture stop already approved logs no `design-check` event.

**GA-E-039** — A check result whose `design-check` event a `change` answer has logged logs no further event when the stop is answered again before the result is replaced (GA-R-171, GA-R-165).

**GA-E-041** — A check that produces no result in a check run counts as reporting no finding in that run.

**GA-E-045** — A check that produces no result in a check run is not respawned before that run ends.

**GA-E-042** — A confirmed abandon while a pass-back awaits the architect's `ready` logs that run's events as the stop's, with `changed=0`, and no round event.

**GA-E-043** — A finding ID in the architect's answer that the passed-back run did not report counts neither as fixed nor as marked.

**GA-E-044** — A finding ID the architect names both fixed and `Needs user` counts as marked only.
