# Git writes — API contract

## ferrule stage commit refusals

- `commit=refused reason=nothing-staged head=<sha> hint=<fix>`
- `commit=refused reason=git hint=<fix>` — the underlying commit failed with changes staged, a refusing hook for example; nothing staged is no longer among its causes.
- `commit=refused reason=rebasing hint=<fix>` — a rebase is in progress in the run's worktree, with and without `--adopt` (GW-R-048).
- The `ferrule stage commit` help lists the `--adopt` refusals of `## ferrule stage commit --adopt`.
- `commit=refused reason=test-unproven tests=<a,b> hint=<fix>` — new tests, each named with its prefix, with no recorded failure, mutation proof or pins-existing mark on the card, with and without `--adopt` (GW-R-069, GW-R-108).
- `commit=refused reason=test-unreplaced tests=<a,b> hint=<fix>` — removed or rewritten tests no `--replaced` pair names (GW-R-070, GW-R-071).
- `--replaced <old>=<new>` — repeatable, with `-m` or `--adopt`, conflicts with `--no-run`; any test name; `<t>=<t>` on a rewritten test needs a proof as a new test does (GW-R-071, GW-R-072, GW-R-076).
- The `--no-run` note of the `ferrule stage commit` help names the new-test and removed-test checks as not applying (GW-E-016).

## ferrule stage commit --adopt

- `ferrule stage commit <card> --adopt <sha>` — conflicts with `-m` and `--no-run` (GW-R-037).
- `commit=done sha=<sha> card=<id>` — as a normal stage commit (GW-R-030).
- Refusals after the shared `reason=main|card|state|branch`, each ending in `hint=<fix>`: `commit=refused reason=dirty` (GW-R-031), `reason=not-head head=<sha>` and `reason=merge parents=<n>` (GW-R-032), `reason=adopted card=<id>` (GW-E-011), `reason=unlogged sha=<sha>` (GW-R-033), `reason=no-merge-base` when the branch point with the base branch cannot be found (GW-R-033), `reason=test-unproven` and `reason=test-unreplaced` as a normal stage commit (GW-E-015), `reason=trailers` (GW-R-035).

## Stage done move

- `card-move=refused reason=unverified from=inreview to=done commit=<sha> hint=<fix>` — `commit=` the stage card's last `commit=`, not verified (GW-R-101, GW-R-102).

## ferrule stage red

- `ferrule stage red <card> <test> [--mutation <text>]` — runs the tests matching `<test>` in the current worktree with the test command its prefix picks, the prefix stripped (GW-R-068, GW-R-096).
- `red=recorded tests=<name>` — one `test-red` logged, for the given test only (GW-R-068, GW-R-073).
- Refusals, each ending in `hint=<fix>`: `red=refused reason=card|state`; `reason=prefix prefixes=<a,b>` (GW-R-106); `reason=no-command`, hint `add a [[test.command]] to ferrule.toml` (GW-E-029); `reason=no-failure` per GW-R-107, GW-E-039, GW-E-034; `reason=run` when the test command cannot start or runs past its timeout (GW-R-099).

## ferrule stage pins

- `ferrule stage pins <card> <test> -- <text>` (GW-R-074).
- `pins=recorded test=<t>`.
- Refusals, each ending in `hint=<fix>`: `pins=refused reason=card|state|no-command|prefix`, `prefixes=<a,b>` with `prefix` (GW-R-110, GW-E-029).

## ferrule.toml [test]

- `[[test.command]]` tables: `prefix` (optional), `run = [<arg>, …]` — program and arguments, run without a shell; `{test}` is replaced by the test name without its prefix inside any argument and must appear in at least one, else `ferrule.toml` is refused when it loads with `test command needs a {test} argument` (GW-R-096, GW-E-031); `failed` (optional regex, GW-R-107); `files = [<glob>, …]` and `declare` (optional regex with a `name` group, matched across a whole test file so it may span lines, `^`/`$` per line), set together (GW-R-108, GW-R-111, GW-E-041–GW-E-043); test bodies end by syntax tree in `.rs` `.py` `.js` `.mjs` `.cjs` `.jsx` `.ts` `.mts` `.cts` `.tsx` `.go` files, by the next declaration elsewhere (GW-R-113, GW-R-109).
- Two or more commands need a prefix each, and no prefix starts another (GW-R-105, GW-E-037); a single unprefixed command takes names as given (GW-R-104).
- `[test] command = [<arg>, …]` still loads as one unprefixed command with neither pattern.
- No default command: with none configured, `ferrule stage red` refuses `reason=no-command` (GW-E-029).
- `timeout = "<n>s|<n>m|<n>h"`, default `30m`, shared by all commands; bounds each `ferrule stage red` run and each stage-check test (GW-R-099, GW-R-100, GA-R-176).

## ferrule pr push refusals

- `push=refused reason=diverged local=<sha> remote=<sha> hint=<fix>`
- `push=refused reason=lease remote=<sha|none> pushed=<sha|none> hint=<fix>` — the remote head is not the one ferrule last pushed (GW-R-051).
- `push=refused reason=rebasing hint=<fix>` — a rebase is in progress in the run's worktree (GW-R-061).

## ferrule pr push output

- `push=done sha=<sha> checks=fast [cached] [forced] …` — only the push-level steps ran (GW-R-078)
- `push=done … forced` — the push was the leased force-push after a rebase (GW-R-050).

## ferrule prompt render blocked

- `--local <sha> --remote <sha>` — each requires the other (GW-R-028).
- The diverged stop offers three options (GW-R-017): `a) rebase through ferrule`, `b) resolved outside ferrule, push again` and `c) stop`.
- `--mismatch <why>` — the refusal's `mismatch=`; renders the sha stop and shows it; a usage error with `--local` or `--remote` (GW-R-087, GW-R-088, GW-R-089).
- The sha stop offers `a) continue with the next stage, push after its commit` (only while a stage is left), `b) fixed outside ferrule, push again` and `c) stop` (GW-R-083, GW-R-084).

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
- `test-red test=<test> [mutation=<text>]` on a stage card, logged only by `ferrule stage red` (GW-R-068, GW-R-073).
- `test-pins test=<test> -- <text>` on a stage card, logged only by `ferrule stage pins` (GW-R-074).
- `test-replaced old=<old> new=<new>`, one per `--replaced` pair, never cut or refused for its length, logged only by `ferrule stage commit` after `commit=` (GW-R-076, GW-R-092).
- `files-outside=<path>`, one per path outside the card's `files` list, logged only by `ferrule stage commit` after `commit=` (GW-R-077).
- `test-checks=skipped reason=no-command|no-detection [prefixes=<a,b>]` on a stage card, one per stage commit, logged only by `ferrule stage commit` (GW-E-017, GW-E-044).
- `pushed=<sha>` on the run card, logged only by `ferrule pr push` (GA-R-075, GW-R-066)
