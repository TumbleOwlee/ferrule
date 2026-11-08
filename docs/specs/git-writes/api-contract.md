# Git writes — API contract

## ferrule stage commit refusals

- `commit=refused reason=nothing-staged head=<sha> hint=<fix>`
- `commit=refused reason=git hint=<fix>` — the underlying commit failed with changes staged, a refusing hook for example; nothing staged is no longer among its causes.
- `commit=refused reason=rebasing hint=<fix>` — a rebase is in progress in the run's worktree, with and without `--adopt` (GW-R-048).
- The `ferrule stage commit` help lists the `--adopt` refusals of `## ferrule stage commit --adopt`.

## ferrule stage commit --adopt

- `ferrule stage commit <card> --adopt <sha>` — conflicts with `-m` and `--no-run` (GW-R-037).
- `commit=done sha=<sha> card=<id>` — as a normal stage commit (GW-R-030).
- Refusals after the shared `reason=main|card|state|branch`, each ending in `hint=<fix>`: `commit=refused reason=dirty` (GW-R-031), `reason=not-head head=<sha>` and `reason=merge parents=<n>` (GW-R-032), `reason=adopted card=<id>` (GW-E-011), `reason=unlogged sha=<sha>` (GW-R-033), `reason=no-merge-base` when the branch point with the base branch cannot be found (GW-R-033), `reason=files path=<path>` (GW-R-034), `reason=trailers` (GW-R-035).

## ferrule pr push refusals

- `push=refused reason=diverged local=<sha> remote=<sha> hint=<fix>`
- `push=refused reason=lease remote=<sha|none> pushed=<sha|none> hint=<fix>` — the remote head is not the one ferrule last pushed (GW-R-051).
- `push=refused reason=rebasing hint=<fix>` — a rebase is in progress in the run's worktree (GW-R-061).

## ferrule pr push output

- `push=done … forced` — the push was the leased force-push after a rebase (GW-R-050).

## ferrule prompt render blocked

- `--local <sha> --remote <sha>` — each requires the other (GW-R-028).
- The diverged stop offers three options (GW-R-017): `a) rebase through ferrule`, `b) resolved outside ferrule, push again` and `c) stop`.

## ferrule run reconcile rows

- `differ row=diverged <branch> local=<sha> remote=<sha>`

## ferrule run rebase

- `ferrule run rebase <slug> [--onto <base>]` — rebases the run's branch onto `<base>`, by default `origin/<main branch>` after a fetch (GW-R-039, GW-R-056).
- `rebase=done head=<sha> mapped=<n>` (GW-R-042).
- `rebase=unchanged head=<sha>` (GW-R-054).
- `rebase=conflict paths=<n> hint=<fix>`, exit 1 (GW-R-041).
- Refusals, each ending in `hint=<fix>`: `rebase=refused reason=card|branch|no-worktree`, `reason=dirty` (GW-R-040), `reason=state card=<id>` (GW-R-053), `reason=in-progress` (GW-R-057), `reason=no-merge-base`, `reason=count old=<n> new=<n>` and `reason=mismatch at=<n> old=<sha> new=<sha>` (GW-R-043), `reason=git detail="<git's error>"`.

## Card events

- `rebase=<head> base=<base> from=<old head>` on the run card, logged only by `ferrule run rebase` (GW-R-042, GW-R-045, GW-R-050).
