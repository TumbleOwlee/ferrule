# Git writes — API contract

## ferrule stage commit refusals

- `commit=refused reason=nothing-staged head=<sha> hint=<fix>`
- `commit=refused reason=git hint=<fix>` — the underlying commit failed with changes staged, a refusing hook for example; nothing staged is no longer among its causes.
- The `ferrule stage commit` help lists the `--adopt` refusals of `## ferrule stage commit --adopt`.

## ferrule stage commit --adopt

- `ferrule stage commit <card> --adopt <sha>` — conflicts with `-m` and `--no-run` (GW-R-037).
- `commit=done sha=<sha> card=<id>` — as a normal stage commit (GW-R-030).
- Refusals after the shared `reason=main|card|state|branch`, each ending in `hint=<fix>`: `commit=refused reason=dirty` (GW-R-031), `reason=not-head head=<sha>` and `reason=merge parents=<n>` (GW-R-032), `reason=adopted card=<id>` (GW-E-011), `reason=unlogged sha=<sha>` (GW-R-033), `reason=no-merge-base` when the branch point with the base branch cannot be found (GW-R-033), `reason=files path=<path>` (GW-R-034), `reason=trailers` (GW-R-035).

## ferrule pr push refusals

- `push=refused reason=diverged local=<sha> remote=<sha> hint=<fix>`

## ferrule prompt render blocked

- `--local <sha> --remote <sha>` — each requires the other (GW-R-028).
- The diverged stop offers two options (GW-R-017): `a) resolved outside ferrule, push again` and `b) stop`.

## ferrule run reconcile rows

- `differ row=diverged <branch> local=<sha> remote=<sha>`
