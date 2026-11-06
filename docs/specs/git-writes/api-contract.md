# Git writes — API contract

## ferrule stage commit refusals

- `commit=refused reason=nothing-staged head=<sha> hint=<fix>`
- `commit=refused reason=git hint=<fix>` — the underlying commit failed with changes staged, a refusing hook for example; nothing staged is no longer among its causes.

## ferrule pr push refusals

- `push=refused reason=diverged local=<sha> remote=<sha> hint=<fix>`

## ferrule prompt render blocked

- `--local <sha> --remote <sha>` — each requires the other (GW-R-028).
- The diverged stop offers two options (GW-R-017): `a) resolved outside ferrule, push again` and `b) stop`.

## ferrule run reconcile rows

- `differ row=diverged <branch> local=<sha> remote=<sha>`
