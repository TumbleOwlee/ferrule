# Gates — API contract

## ferrule gate approve flags

- `gate2 --live-repo <owner/name>` names the live-check repository (GA-R-062)
- `gate2 --tiers s<n>=<own|batched|skipped>,…` sets the tier of each stage named; lists only the overrides (GA-R-070)
- `approve=refused reason=tier stage=s<n> risk=<host-writes|persisted-state|public-surface> hint=<fix>` — a skipped tier on a stage with a risk; nothing is written (GA-R-071)
- `approve=refused reason=tier stage=s<n> risk=<word> hint=<fix>` — a plan Risk word that names no risk; nothing is written (GA-E-014)

## ferrule stage tier

- `ferrule stage tier <slug> s<n> [--after-blocker]` prints `tier=<own|batched|skipped> review=<stage|batch|none> [stages=s<a>,…] [promoted=spec-change|batch-blocker]` (GA-R-072, GA-R-073, GA-R-074, GA-R-086, GA-R-087, GA-R-088)

## ferrule ci gauntlet

- `--level <push|review|full>`, default full; `--stage s<n>` also runs the tests the stage's diff adds or rewrites (GA-R-082, GA-R-083, GA-R-084)
- `[[gauntlet.step]] at = [<level>, …]` names the levels a step runs at besides full; `{crates}` in `cmd` expands per level (GA-R-091, GA-E-009, GA-E-011)

## Reviewer status line

- `blockers=<n>` and `minors=<n>` on the stage and batch status line, `minors=<n>` on the branch status line (GA-R-092, GA-R-094)

## ferrule gate next

- `ferrule gate next <slug> stage <status line>` takes `clean` and `findings count=<n>` (GA-R-099, GA-R-095)
- `next=minors then=<stage-stop|stop|pr> [reason=<why>]`, followed by the `run:`/`do:`/`guide:` lines of the step after the fix (GA-R-095, GA-R-096, GA-R-097)
- `usage: ferrule gate next <slug> <stage|gate3> status=<word>…` (exit 2) for a clean line without `minors=` (GA-R-100)

## Card events

- `tier=<tier> [promoted=<why>]` on a stage card, logged only by `ferrule stage tier` (GA-R-072, GA-R-073)
- `gauntlet=pass|fail` on the run card, logged by `ferrule ci gauntlet` at level full (GA-R-090)
- `review=clean scope=stage [minors=<n>]`, `review=clean [minors=<n>]` and `review=<n>-resolved [minors=<n>]` on the run card, logged by `ferrule gate next` (GA-R-101, GA-R-103)
