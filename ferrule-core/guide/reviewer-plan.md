# reviewer-plan — Plan and re-plan checks

Read by `fe-reviewer` in scope `plan` or `plan from s<n>`.

<!-- include: concise -->

<!-- include: reviewer-start -->

## Scope

Scope token from caller: `plan` or `plan from s<n>`. Never widen.

<!-- include: reviewer-inputs -->

## Plan checks

Check `plan.md` against `spec-diff.md`, spec-fidelity axis only:
- quoted requirement text matches
- appended IDs unused in `docs/specs/`
- nothing contradicts an existing requirement
- every ID has a stage and a test
- `s0` lands every ID (no `s0` when `spec-diff.md` has no entries to land, or there is no `spec-diff.md`)
- no local `R<n>`/`E<n>` left; every fixed ID new to the run is unused on `{main}` (`ferrule spec refs <ID>… --ref {main} --path docs/specs` answers `refs=none`) and sequential within its prefix
- every scenario `S<n>` is pinned by a test or a stage's Verification
- the plan reaches `architecture.md`: every new or changed component has a stage, no stage builds one it doesn't name, and every signature of what `## Public surface` lists appears as written in the step that builds it
- spec edits the planner made since the architecture stop are recorded (`Revisions:`, a `decisions.md` line each)
- every stage title is imperative, verb first; ≤ 3 words, ≤ 20 characters; letters, digits, hyphens, spaces only; unique within the plan; no stage id; `s0`, when present, is always `Land spec`; and it matches its heading and stage-table row

## Re-plan checks

`plan from s<n>` — a re-plan after code landed: `ferrule run plan-diff {slug}` (`reason=no-prev`, a rebuilt board: read `plan.md` whole and the committed stages' cards instead of a diff). Any change to a section before `s<n>`, to `spec-diff.md` or a fixed ID is a blocker. Every rewritten stage gets the checks above that concern stages; every ID and scenario a replaced stage pinned is pinned by a rewritten stage or a committed one; a step that changes the spec names the entry it changes.

## Follow-up pass

<!-- include: reviewer-follow-up-start -->
- `ferrule run plan-diff {slug}` (and the `spec-diff.md` sections the revision touched). Every changed section gets every check of this guide, not only the lines your findings named.
<!-- include: reviewer-follow-up-end -->

<!-- include: reviewer-output -->
