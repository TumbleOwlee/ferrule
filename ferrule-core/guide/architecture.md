# architecture — The architecture step

## Architecture

`fe-architect` reads the code, grills the user on structure and drafts `architecture.md` (`ferrule guide show architect` holds the shape); user decides, orchestrator relays. Spawn it with: `{run_dir}/spec-diff.md` path (non-behavior change: the goal verbatim instead), affected area(s), anything the user volunteered at gate 1.

- `status=question` → `ferrule guide show question`.
- `status=split` → `ferrule guide show split`.
- `status=behavior-change` (non-behavior run) → `ferrule guide show principles --section 'Principles — implementation'`.
- `status=no-arch` → `ferrule board set {slug} arch=no-arch`; no stop; gate 2 (fast lane: its stop approves the brief).
- `status=ready slices=…` → the user chose an epic at the architect's size guard: `ferrule board set {slug} kind=epic lane=`; `ferrule guide show epic` step 3.
- `status=ready` → `ferrule prompt render {slug} arch` (shows `architecture.summary.md`, `architecture.md` with the interface sketches, and the brief when still pending — fast lane — or moved; a non-behavior run's first stop offers the lane). Change requested → relay to the same agent.
  - Approved → `ferrule gate approve {slug} arch` (records `gate1` too while it is pending — fast lane); `ferrule guide show gate2`. Fast lane: the approval pre-authorizes gate 2's *On approval* once the plan review is clean; next stop gate 4.
- The agent stays alive until gate 2's approval, for the planner's `arch-gap`; a resumed run spawns a fresh one, which re-reads `architecture.md`.
