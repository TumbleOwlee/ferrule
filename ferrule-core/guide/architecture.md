# architecture — The architecture step

## Architecture

`fe-architect` reads the code, grills the user on structure and drafts `architecture.md`; user decides, orchestrator relays. Spawn it with: `{run_dir}/spec-diff.md` path (non-behavior change: the goal verbatim instead), affected area(s), anything the user volunteered at gate 1.

- `status=question` → `ferrule guide show question`.
- `status=split` → `ferrule guide show split`.
- `status=behavior-change` (non-behavior run) → `ferrule guide show principles --section 'Principles — implementation'`.
- `status=no-arch` → `ferrule board set {slug} arch=no-arch`; no stop; gate 2 (its stop approves the brief while `gate1` is pending).
- `status=ready slices=…` → the user chose an epic at the architect's size guard: `ferrule board set {slug} kind=epic`; `ferrule guide show epic` step 3.
- `status=ready` → `ferrule prompt render {slug} arch` (shows `architecture.summary.md`, `architecture.md` with the interface sketches, and the brief when still pending or moved). Change requested → relay to the same agent.
  - Approved → `ferrule gate approve {slug} arch` (records `gate1` too while it is pending); `ferrule guide show gate2`. Option 1, *stop at the plan too*: `--plan-stop`; without it the approval pre-authorizes gate 2's *On approval* once the plan review is clean.
- The agent stays alive until gate 2's approval, for the planner's `arch-gap`; a resumed run spawns a fresh one, which re-reads `architecture.md`.
