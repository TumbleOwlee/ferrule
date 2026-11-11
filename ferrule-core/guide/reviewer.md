# reviewer — Reviewing a plan, a stage or the branch

<!-- include: concise -->

<!-- include: reviewer-start -->

## Scope

Scope token from caller: `plan` (no diff; `## Plan scope`), `plan from s<n>` (a re-plan; `## Plan scope`), `stage s<n>` (base = previous stage's commit), `batch s<a>,s<b>` (base = the commit before `s<a>`; reads the commits of every listed stage and the uncommitted fixes, like `stage s<n>`), `delta s<n>` (base = previous stage's commit; only `ferrule run diff {slug} --base <base> --specs` and the card's `spec-delta` lines — `## Spec changes`, nothing else), `branch` (gate 3 — cross-stage bugs live here). Never widen. Caller names the stage ids in scope.

**Scope `stage s<n>` or `branch`:** a full-lane stage reaches this review uncommitted, and `--base` omits it. Read both: `ferrule run diff {slug} --base <base>` and `ferrule run diff {slug} --uncommitted` (empty once the work is committed). `--uncommitted` lists untracked files without diffing them: read every `??` path whole, each file of a listed directory too. Only when both are empty: `status=blocked reason=empty diff`. Scope `batch s<a>,s<b>` reads the same two, its base the commit before `s<a>`.

<!-- include: reviewer-inputs -->

## Plan scope

Scope `plan` or `plan from s<n>`: `ferrule guide show reviewer-plan` replaces this guide; it holds the checks.

## Axes, reported separately

**Spec fidelity** — every requirement implemented as written in the branch's `docs/specs/` (quote requirement + satisfying code path); nothing beyond it (scope creep is a finding even if good code); every new ID pinned by a test that genuinely exercises it (citing an ID but asserting something else is worse than none); every scenario walked by its pinning test or Verification; every spec change on the branch classified (`## Spec changes`). The stage card's notes (`ferrule board show {slug}.s<n>`) — `files-outside=`, `test-pins` and `test-replaced` lines — point at changes worth a look; a finding may concern such a change, the note itself never blocks and never reaches a user stop.

**Architecture conformance** — the code builds the components of `architecture.md` with their ownership, interactions, interfaces and public surface. A sketch in `## Interfaces` binds only what `## Public surface` lists — a difference there is a deviation, any other difference from a sketch is none. A deviation from a `(user)` decision, a user-approved component or the public surface → `Needs user` (a) keep — the architect revises `architecture.md` to match; b) revert to the approved structure — with your recommendation). Any other deviation → major, fix: conform, or name it as a plan amendment.

**Standards** — every rule `AGENTS.md` states, under any heading; test naming; the ID citation rule below; unflagged semver-relevant public surface changes. Comment severity: a comment rule the project states sets its own severity, otherwise a minor.

<!-- include: id-citation -->

**Docs currency** — top-level docs the diff's behavior touches still match: README.md (flags, config keys, protocols/modes, setup), ARCHITECTURE.md (crate graph, data flow, concurrency), PRD.md (scope), CONTRIBUTING.md (workflow). Scoped to what this change affects, not a full audit. Stale doc = finding.

## Spec changes

Every change to `docs/specs/` after `s0` — `ferrule run diff {slug} --base <base> --specs` against the card's `spec-delta` lines; an unlogged change is a major. Scopes `stage s<n>` and `branch` also read `ferrule run diff {slug} --uncommitted --specs`; a `??` path under `docs/specs/` there is a change too. No cards (caller says so): skip the log check; every branch spec text differing from the caller's approved baseline is a change to classify. Check each against `docs/specs/README.md` rules (a violation is a standards finding), then classify:

- **meaningful** — contradicts a scenario or a `(user)` line of `decisions.md`; or widens or narrows what a user sees beyond a `## Deferred` item; or reaches outside the goal. One `Needs user` item each: the entry's approved text, the new text, a) keep b) revert c) split (only when outside the goal), with your recommendation.
- **detail** — everything else: fills a deferred point, sharpens wording, splits an entry, adds an edge case consistent with the scenarios. Listed on the verdict's `Spec changes` line; no decision.

## Follow-up pass

<!-- include: reviewer-follow-up-start -->
- `plan` scope: `ferrule run plan-diff {slug}` (and `spec-diff.md`'s sections the revision touched). Every changed section gets every check of `ferrule guide show reviewer-plan`, not only the lines your findings named — a revision breaks things elsewhere as often as it fixes the finding.
- `stage s<n>` / `batch s<a>,s<b>` / `delta s<n>` scope: the stage's current diff (the same commands as the first pass) whole — a stage diff is small; your earlier findings don't narrow it.
<!-- include: reviewer-follow-up-end -->

<!-- include: reviewer-output -->
