# Git writes

## Nothing staged

**GW-R-001** — `ferrule stage commit` with nothing staged refuses with `reason=nothing-staged head=<sha>`, never `reason=git`.

**GW-R-002** — The nothing-staged refusal on a dirty worktree, untracked files included, has a hint saying `git add` was forgotten.

**GW-R-003** — The nothing-staged refusal on a clean worktree has a hint saying the changes may already be committed.

**GW-R-004** — `ferrule stage commit --no-run` applies the nothing-staged check.

**GW-R-005** — The `ferrule stage commit` help lists the nothing-staged refusal.

**GW-R-006** — The implementer guide directs a clean-worktree nothing-staged refusal to `status=blocked` with `head=` and a suggestion of `ferrule stage commit <card> --adopt <head>` in the reason, and an `--adopt` refusal for a file outside the `files` list to a re-plan.

**GW-R-026** — The `--no-run` note of the `ferrule stage commit` help names the nothing-staged check as applying.

## Adopted commit

**GW-R-030** — `ferrule stage commit <card> --adopt <sha>` logs `commit=<sha>` on the card as a normal stage commit would, creating no commit and leaving the commit's message and history unchanged.

**GW-R-031** — `--adopt` refuses unless the card is `inreview`, the worktree is clean and it is on the run's branch.

**GW-R-032** — `--adopt` refuses unless `<sha>` is the branch's HEAD with exactly one parent.

**GW-R-033** — `--adopt` refuses unless every commit from the branch point with the base branch up to HEAD^ is logged as `commit=` on one of the run's cards.

**GW-R-034** — `--adopt` refuses when the commit touches a file outside the card's `files` list, counting both sides of a rename.

**GW-R-035** — `--adopt` refuses a commit whose message carries attribution trailers, with a hint to amend the commit if it has not been pushed.

**GW-R-036** — An adopted commit goes through the same push checks, gauntlet and stage review as a commit made by `ferrule stage commit`.

**GW-R-037** — `ferrule stage commit` requires `-m` unless `--adopt` is given, and `--adopt` together with `-m` or `--no-run` is a usage error.

**GW-R-038** — The stages guide routes a nothing-staged blocked reason to the blocked stop with its standard options, so answering `a` resumes the stage to adopt the commit.

## Diverged push

**GW-R-007** — `ferrule pr push` refuses with `local=` and `remote=` when the remote branch's head is not an ancestor of the local head, unless a rebase was recorded in the run since its last push.

**GW-R-008** — Only exit code 1 from `git merge-base --is-ancestor`, run after `git fetch`, counts as diverged, never git's error text.

**GW-R-009** — The divergence check runs before the gauntlet, so a diverged refusal runs no gauntlet.

**GW-R-010** — A branch that `git ls-remote --exit-code` reports missing from the remote is pushed as a normal first push.

**GW-R-011** — A divergence check that cannot complete for any reason lets the gauntlet and the push go on.

**GW-R-012** — `ferrule pr push --no-run` applies the same divergence check and gives the same refusal fields.

**GW-R-013** — The diverged refusal's hint names the rebase command as the way to resolve it in a run and says never to force-push by hand.

**GW-R-021** — In a run, the `reason=dirty` and `reason=sha` refusals take precedence over the diverged refusal.

**GW-R-022** — A divergence check still unfinished after 60 seconds counts as one that cannot complete.

**GW-R-025** — A divergence check runs with git's terminal credential prompt turned off, so a remote asking for credentials counts as one that cannot complete.

**GW-R-029** — A divergence check that reaches the 60-second limit of GW-R-022 leaves none of the processes it started running once it returns.

## Diverged routing

**GW-R-014** — The stages guide sends a diverged refusal to the blocked stop and suggests no re-plan.

**GW-R-015** — The blocked stop for a diverged refusal shows the `local=` and `remote=` heads.

**GW-R-016** — The blocked stop for a diverged refusal says to rebase through ferrule or resolve it outside ferrule, and never to force-push by hand.

**GW-R-017** — The blocked stop for a diverged refusal offers only `a) rebase through ferrule`, `b) resolved outside ferrule, push again` and `c) stop`.

**GW-R-018** — `ferrule run reconcile` reports a diverged branch as a `differ` row with `local=` and `remote=`, using the same check as `ferrule pr push`, unless a rebase was recorded in the run since its last push.

**GW-R-019** — `ferrule run reconcile` adds no diverged row when the check cannot complete.

**GW-R-020** — The resume guide states that a diverged row is resolved with the rebase command or outside ferrule, never by a force-push by hand.

**GW-R-023** — `ferrule run reconcile --local` does not run the divergence check.

**GW-R-024** — `ferrule prompt render blocked` renders the diverged stop when given `--local` and `--remote`.

**GW-R-027** — The diverged stop names no stage.

**GW-R-028** — `ferrule prompt render blocked` given only one of `--local` and `--remote` is a usage error.

## Rebased branch

**GW-R-039** — A ferrule command rebases the run's branch onto a named base, moving only the commits from the branch point with the base branch up to HEAD.

**GW-R-040** — The rebase command refuses when the run's worktree has uncommitted changes, untracked files included.

**GW-R-041** — A rebase that hits a conflict is left in progress, and the command's output says to resolve the conflict, stage the files and run `git rebase --continue`, then run the rebase command again.

**GW-R-042** — Once the rebase completes, every commit logged as `commit=` on a stage card of the run between the branch point and the old head is mapped to the rebased commit at the same position, and that commit is appended to the same card as a new `commit=`, in the old order, its old `commit=` lines kept.

**GW-R-043** — The mapping refuses, logging no `commit=`, when the commits from the branch point with the base up to the newest `commit=` logged on the run's cards differ from those up to the rebased HEAD in count, order or commit message.

**GW-R-044** — Only committed stages are mapped; a stage not yet committed commits on the rebased branch as usual.

**GW-R-045** — A mapped rebase makes the last `gauntlet=pass` no longer count, so the next push reruns the gauntlet on the rebased head.

**GW-R-046** — A mapped rebase leaves the stages' approved reviews in place.

**GW-R-047** — A rebase aborted with `git rebase --abort` leaves the branch and the stage cards as before and logs nothing.

**GW-R-048** — `ferrule stage commit <card>` refuses while a rebase is in progress in the run's worktree, with and without `--adopt`.

**GW-R-049** — `ferrule run reconcile` checks each stage's last `commit=`, so it reports clean after a mapped rebase.

**GW-R-050** — `ferrule pr push` in a run with a rebase recorded since its last push force-pushes with a lease on the remote head ferrule last pushed.

**GW-R-051** — The force-push refuses, pushing nothing, when the remote head is not the one ferrule last pushed.

**GW-R-052** — The resume guide names the rebase command for a run whose PR conflicts with its base.

**GW-R-053** — The rebase command refuses, changing nothing, while a stage card of the run is `inprogress`.

**GW-R-054** — The rebase command logs nothing and says the branch is unchanged when the newest `commit=` logged on the run's cards is already on the branch after the rebase.

**GW-R-055** — `ferrule pr push --no-run` never force-pushes.

**GW-R-056** — Without a named base, the rebase command fetches the main branch from origin and rebases onto `origin/<main branch>`.

**GW-R-057** — The rebase command refuses, changing nothing, while a rebase is already in progress in the run's worktree, with a hint to resolve and continue it or abort it.

**GW-R-058** — The stages guide sends a rebase stopped on a conflict to the implementer, briefed with the card of the stage whose `commit=` is the newest on the run's cards.

**GW-R-059** — The implementer guide has an implementer briefed with a rebase conflict edit only the files git reports as conflicted, stage them, run `git rebase --continue` and report `status=rebased`.

**GW-R-060** — The stages guide answers `status=rebased` by running the rebase command again, then `ferrule pr push`.

**GW-R-061** — `ferrule pr push` in a run refuses while a rebase is in progress in the run's worktree.

**GW-R-062** — In a run with a rebase recorded since its last push, a remote-head check that cannot complete lets the gauntlet and the leased force-push go on.

**GW-R-064** — After a mapped rebase, `ferrule run status` prints `gauntlet=-` until a later gauntlet line is logged.
