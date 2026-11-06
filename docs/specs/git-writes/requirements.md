# Git writes

## Nothing staged

**GW-R-001** — `ferrule stage commit` with nothing staged refuses with `reason=nothing-staged head=<sha>`, never `reason=git`.

**GW-R-002** — The nothing-staged refusal on a dirty worktree, untracked files included, has a hint saying `git add` was forgotten.

**GW-R-003** — The nothing-staged refusal on a clean worktree has a hint saying the changes may already be committed.

**GW-R-004** — `ferrule stage commit --no-run` applies the nothing-staged check.

**GW-R-005** — The `ferrule stage commit` help lists the nothing-staged refusal.

**GW-R-006** — The implementer guide directs a clean-worktree nothing-staged refusal to `status=blocked` with `head=` in the reason.

**GW-R-026** — The `--no-run` note of the `ferrule stage commit` help names the nothing-staged check as applying.

## Diverged push

**GW-R-007** — `ferrule pr push` refuses with `local=` and `remote=` when the remote branch's head is not an ancestor of the local head.

**GW-R-008** — Only exit code 1 from `git merge-base --is-ancestor`, run after `git fetch`, counts as diverged, never git's error text.

**GW-R-009** — The divergence check runs before the gauntlet, so a diverged refusal runs no gauntlet.

**GW-R-010** — A branch that `git ls-remote --exit-code` reports missing from the remote is pushed as a normal first push.

**GW-R-011** — A divergence check that cannot complete for any reason lets the gauntlet and the push go on.

**GW-R-012** — `ferrule pr push --no-run` applies the same divergence check and gives the same refusal fields.

**GW-R-013** — The diverged refusal's hint says the user must resolve it outside ferrule until ferrule supports rebasing a run's branch, and must never force-push.

**GW-R-021** — In a run, the `reason=dirty` and `reason=sha` refusals take precedence over the diverged refusal.

**GW-R-022** — A divergence check still unfinished after 60 seconds counts as one that cannot complete.

**GW-R-025** — A divergence check runs with git's terminal credential prompt turned off, so a remote asking for credentials counts as one that cannot complete.

## Diverged routing

**GW-R-014** — The stages guide sends a diverged refusal to the blocked stop and suggests no re-plan.

**GW-R-015** — The blocked stop for a diverged refusal shows the `local=` and `remote=` heads.

**GW-R-016** — The blocked stop for a diverged refusal says to resolve it outside ferrule and never to force-push.

**GW-R-017** — The blocked stop for a diverged refusal offers only `a) resolved outside ferrule, push again` and `b) stop`.

**GW-R-018** — `ferrule run reconcile` reports a diverged branch as a `differ` row with `local=` and `remote=`, using the same check as `ferrule pr push`.

**GW-R-019** — `ferrule run reconcile` adds no diverged row when the check cannot complete.

**GW-R-020** — The resume guide states that a diverged row is resolved outside ferrule, never by a force-push.

**GW-R-023** — `ferrule run reconcile --local` does not run the divergence check.

**GW-R-024** — `ferrule prompt render blocked` renders the diverged stop when given `--local` and `--remote`.

**GW-R-027** — The diverged stop names no stage.

**GW-R-028** — `ferrule prompt render blocked` given only one of `--local` and `--remote` is a usage error.
