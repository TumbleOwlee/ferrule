# gate1 — The brief: grilling, lane, approval

## Gate 1 — brief

`fe-author` grills the user and drafts the brief, `spec-diff.md` (`ferrule guide author` holds the grilling rules and the shape); user decides, orchestrator relays. Best effort: scope, scenarios, the user-visible flow and the edge cases the user cares about; everything else lands under `## Deferred` with an owner. Spawn `fe-author` with: user's goal verbatim, affected area(s), `{run_dir}/`, and the issue number when the user named one ("take issue #n") — the request and the `ferrule guide issue` reuse candidate.

- **Board:** `ferrule run init {slug} [--issue <n>]` before spawning: the parent card and `{run_dir}/`; with a named issue also its body (`request.md`) and the brief it may already carry (`issue-spec.md`). `init=done … kind=slice epic=<e>` → the issue is a slice: `ferrule guide epic` first.
- `status=question` → relay question and answer without comment.
- `status=ready size=epic` → always a stop, whatever the lane would be: `ferrule prompt render {slug} gate1 --epic --reason <the author's reason>`. Option 1 (epic) → `ferrule run approve {slug} gate1 --epic`; `ferrule guide epic`. Option 2 or 3 (single run) → as below, with that lane.
- `status=ready` or `status=no-diff` (spec already covers the goal; the file names the violated requirements) → recommend the lane by `count=` (`ferrule guide lanes`):
  - **full** recommended → `ferrule prompt render {slug} gate1`: approval **and lane** in one prompt. Change requested → relay to the same agent. Approved → `ferrule run approve {slug} gate1 --lane <the lane>`.
  - **fast** recommended → no stop: `ferrule card set {slug} lane=fast gate1=with-arch`; at most 1 entry or `no-diff`: `ferrule card set {slug} lane=fast gate1=with-plan arch=skipped`, then `ferrule guide gate2`. Else the brief is approved at the architecture stop.
  - Then `ferrule guide architecture` (skipped: `ferrule guide gate2`).
- The agent stays alive for `ferrule guide issue`, `ferrule guide spec-changes` and gate 4.
