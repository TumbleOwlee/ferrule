# gate1 — The brief: grilling, lane, approval

## Gate 1 — brief

`spec-author` grills the user and drafts the brief, `spec-diff.md` (`ferrule guide author` holds the grilling rules and the shape); user decides, orchestrator relays. Best effort: scope, scenarios, the user-visible flow and the edge cases the user cares about; everything else lands under `## Deferred` with an owner. Spawn `spec-author` with: user's goal verbatim, affected area(s), `{run_dir}/`, and the issue number when the user named one ("take issue #n") — the request and the *Tracking issue* (`ferrule guide issue`) reuse candidate.

- **Board:** `ferrule run init {slug} [--issue <n>]` before spawning: the parent card and `{run_dir}/`; with a named issue also its body (`request.md`) and the brief it may already carry (`issue-spec.md`). `init=done … kind=slice epic=<e>` → the issue is a slice: *Slice run* (`ferrule guide epic`) first.
- `status=question` → relay question and answer without comment.
- `status=ready size=epic` → always a stop, whatever the lane would be: `ferrule prompt render gate1 {slug} size=epic reason=<the author's reason>`. Option 1 (epic) → `ferrule run approve gate1 {slug} kind=epic`; *Epic* (`ferrule guide epic`). Option 2 or 3 (single run) → as below, with that lane.
- `status=ready` or `status=no-diff` (spec already covers the goal; the file names the violated requirements) → recommend the lane by `count=` (*Lane — fast or full* (`ferrule guide lanes`)):
  - **full** recommended → `ferrule prompt render gate1 {slug}`: approval **and lane** in one prompt. Change requested → relay to the same agent. Approved → `ferrule run approve gate1 {slug} lane=<the lane>`.
  - **fast** recommended → no stop: `ferrule card set {slug} lane=fast gate1=with-arch`; at most 1 entry or `no-diff`: `ferrule card set {slug} lane=fast gate1=with-plan arch=skipped`, then *Gate 2* (`ferrule guide gate2`). Else the brief is approved at the architecture stop.
  - Then *Architecture* (`ferrule guide architecture`) (skipped: *Gate 2* (`ferrule guide gate2`)).
- The agent stays alive for *Tracking issue* (`ferrule guide issue`), *Spec changes* (`ferrule guide spec-changes`) and gate 4.
