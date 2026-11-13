# gate1 — The brief: grilling, early stops, approval

## Gate 1 — brief

`fe-author` grills the user and drafts the brief, `spec-diff.md`; user decides, orchestrator relays. Best effort: scope, scenarios, the user-visible flow and the edge cases the user cares about; everything else lands under `## Deferred` with an owner. Spawn `fe-author` with: user's goal verbatim, affected area(s), `{run_dir}/`, and the issue number when the user named one ("take issue #n") — the request and the `ferrule guide show issue` reuse candidate.

- **Board:** `ferrule run preflight` first: `preflight=refused` → stop, relay its `hint=` to the user, no card is made; then `ferrule run init {slug} [--issue <n>]` before spawning: the parent card and `{run_dir}/`; with a named issue also its body (`request.md`) and the brief it may already carry (`issue-spec.md`). `init=done … kind=slice milestone=<m> slice=<k>` (without milestones: `kind=slice epic=<e>`) → the issue is a slice: `ferrule guide show epic` first. `init=refused reason=blocked issue=<n> blockers=<list> [unreadable=<k>]`: the slice has open blockers; ask the user, naming the blockers and any `unreadable` count (blockers the host does not show). Approved → rerun `ferrule run init {slug} --issue <n> --start-blocked`; declined → no run (the refusal left no card).
- `status=question` → relay question and answer without comment.
- `status=ready`, `status=no-diff` (spec already covers the goal; the file names the violated requirements) or `status=ready size=epic` (the brief is too big for one run) → `ferrule gate next {slug} gate1 <status line>`; its `next=` decides by the brief's size:
  - `next=stop` → `ferrule prompt render {slug} gate1` (an epic: `--epic --reason <the author's reason>`). Change requested → relay to the same agent. Approved → `ferrule gate approve {slug} gate1`; an epic, option 1 → `ferrule gate approve {slug} gate1 --epic` (records `kind: epic`; refused once the run is past gate 1) and `ferrule guide show epic`, option 2 (single run) → as a plain stop. Then `ferrule guide show architecture`.
  - `next=arch` → no stop: spawn the architect (`ferrule guide show architecture`); the brief is approved at the architecture stop, its option 1 (stop at the plan too) is `ferrule gate approve {slug} arch --plan-stop`.
  - `next=plan` → no stop: run the printed `ferrule board set` (`arch=skipped`), then `ferrule guide show gate2`; the brief is approved at gate 2's stop.
- The agent stays alive for `ferrule guide show issue`, `ferrule guide show spec-changes` and gate 4.
