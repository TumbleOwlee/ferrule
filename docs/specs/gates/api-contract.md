# Gates — API contract

## ferrule gate approve flags

- `gate2 --live-repo <owner/name>` names the live-check repository (GA-R-062)
- `gate2 --tiers s<n>=<own|batched|skipped>,…` sets the tier of each stage named; lists only the overrides (GA-R-070)
- `approve=refused reason=tier stage=s<n> risk=<host-writes|persisted-state|public-surface> hint=<fix>` — a skipped tier on a stage with a risk; nothing is written (GA-R-071)

## ferrule stage tier

- `ferrule stage tier <slug> s<n> [--after-blocker]` prints `tier=<own|batched|skipped> review=<stage|batch|none> [stages=s<a>,…] [promoted=spec-change|batch-blocker]` (GA-R-072, GA-R-073, GA-R-074, GA-R-086, GA-R-087, GA-R-088)

## ferrule ci gauntlet

- `--level <push|review|full>`, default full; `--stage s<n>` also runs the tests the stage's diff adds or rewrites (GA-R-082, GA-R-083, GA-R-084)
- `[[gauntlet.step]] at = [<level>, …]` names the levels a step runs at besides full; `{crates}` in `cmd` expands per level (GA-R-091, GA-E-009, GA-E-011)

## Card events

- `tier=<tier> [promoted=<why>]` on a stage card, logged only by `ferrule stage tier` (GA-R-072, GA-R-073)
- `gauntlet=pass|fail` on the run card, logged by `ferrule ci gauntlet` at level full (GA-R-090)
