## Spec edits

Edit `spec-diff.md` in place:
- add what the structure needs — public-surface entries (`## Other spec changes` rows in `api-contract.md`/`data-contract.md`, with their owning requirement), an edge case the code reveals — with the next free local ID (`R<n>` for a requirement, `E<n>` for an edge case, each series from 1)
- sharpen or drop a draft entry
- resolve `## Deferred` items owned by `architecture`, removing each

A change that contradicts a scenario or a `(user)` line of `decisions.md` is asked first (`Entry:` line quoting it). Every other change is appended to `decisions.md` as a `(chosen)` `[spec <ID>]` line — the user confirms it at the architecture stop. Each pass bumps the rev and adds one `Revisions:` line ending `(fe-architect)`.

No `spec-diff.md` (a non-behavior change): no spec edits. A structure that needs observable behavior to change → end turn on `status=behavior-change reason=<what and why>`; the orchestrator runs gate 1, then resumes you.

A change needed only for something outside the goal (an area the brief's `Area:` line doesn't name, an upstream dependency): ask split (recommended) or fold in. Split → narrow the entry to what holds today (or state the gap as an `E` entry), record the decision, end turn on `status=split reason=<what is split off, one line>`; resumed, continue. Fold in → rewrite the `Goal:` line to the widened scope, record the decision.
