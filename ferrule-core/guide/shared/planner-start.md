## Project rules

`AGENTS.md` is already in your context; never re-read it. The rules it states bind you here. Never read the orchestrator's topics — gate and board mechanics are its own. Then the affected area's files in `docs/specs/<area>/` (`requirements.md`, `edge-cases.md`, `api-contract.md`/`data-contract.md`): the headings the entries of `spec-diff.md` land in or cite.

## Input

Brief: path of `{run_dir}/spec-diff.md` (none for a non-behavior change), path of `{run_dir}/architecture.md` (none when the architect answered `no-arch`), affected area(s), anything the user volunteered; a slice run also gets `{run_dir}/epic.md` — orientation only, `architecture.md` already holds what binds; local IDs you add start at the slice number times 100. Nothing else. No issue/PR/tracker knowledge, ever — never reference one.

`arch=skipped`: no `architecture.md` — read the code yourself and quote inline; a type, trait, enum, module or public surface the change needs → `status=arch-gap`.

Read both files whole. `architecture.md` is the target: every new or changed component gets a stage, no stage builds a component it doesn't name. Start from its `## Code map`: its excerpts are verbatim — cite them, never copy them into steps, never re-read a file to confirm one. Read code only for what the map does not hold. Build to the signatures in its `## Interfaces`; bodies, private helpers and stage order are yours. A structural change the plan needs (a component added, split or dropped, a signature in `## Interfaces` changed, a public-surface change, different ownership) is not yours: end turn on `status=arch-gap reason=<what and why>`, stay running; the orchestrator resumes `fe-architect`, then you — re-read only its changed sections.

## Interview before drafting

Surface every plan-shaped decision (stage boundaries, test strategy, file layout — structure is settled in `architecture.md`) via the orchestrator, with a recommendation, in `{run_dir}/question.md` (`## Questions`) and end turn on `status=question file=… count=<n>`. No plan until every decision is resolved.

Every decision, asked or not, is one line appended to `decisions.md` (`## Decision lines`).

**Non-behavior run** (no `spec-diff.md`): no spec edits; a plan that needs observable behavior to change → `status=behavior-change reason=<what and why>`, stay running; the orchestrator runs gate 1, then resumes you.

**Spec edits.** Planning shows an entry wrong, coarse or missing: edit `spec-diff.md` in place — add with the next free local ID (`R<n>` for a requirement, `E<n>` for an edge case, each series from 1), sharpen, split so each entry holds one rule (one subject, one condition, one outcome), or drop. Resolve `## Deferred` items owned by `plan`, removing each. A change that contradicts a scenario or a `(user)` line of `decisions.md` goes into the question batch first (`Entry:` line quoting it); every other change is appended to `decisions.md` as a `(chosen)` `[spec <ID>]` line. Each pass bumps the rev and adds one `Revisions:` line ending `(fe-planner)`. A change needed only for something outside the goal: ask split (recommended) or fold in; split → narrow the entry, record the decision, end turn on `status=split reason=<what is split off, one line>`, continue when resumed.

**Fixed IDs** — last, once every decision is settled. Per local `R`/`E` entry, the next free ID of its area's prefix and series: `ferrule spec next <PREFIX> --count <n>` (`--edge` for `-E-`) hands out the next `n`, counting up from the highest on `{main}` — never the working tree, which may be another branch. Rewrite the heading `## <PREFIX>-R-nnn — new · <file> `## <heading>` · was R<n>` and every reference to the local ID in `spec-diff.md` and `plan.md`; the ` · was R<n>` suffix stays in `spec-diff.md` through gate 4 — the PR's Issue column reads it. Retired local IDs get none. Scenarios keep `S<n>` — they never land. After this no `R<n>`/`E<n>` remains anywhere; tests and the implementer cite fixed IDs.

**Area docs unwieldy** (`requirements.md`/`edge-cases.md` costs real context): one line in `## Shared`, never act on it — splitting an area is a gate 1 decision.

## Questions

Title line of `question.md`: `# {slug} — questions (gate 2)`.
