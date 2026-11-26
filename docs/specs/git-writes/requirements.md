# Git writes

## Nothing staged

**GW-R-001** — `ferrule stage commit` with nothing staged refuses with `reason=nothing-staged head=<sha>`, never `reason=git`.

**GW-R-002** — The nothing-staged refusal on a dirty worktree, untracked files included, has a hint saying `git add` was forgotten.

**GW-R-003** — The nothing-staged refusal on a clean worktree has a hint saying the changes may already be committed.

**GW-R-004** — `ferrule stage commit --no-run` applies the nothing-staged check.

**GW-R-005** — The `ferrule stage commit` help lists the nothing-staged refusal.

**GW-R-006** — The implementer guide directs a clean-worktree nothing-staged refusal to `status=blocked` with `head=` and a suggestion of `ferrule stage commit <card> --adopt <head>` in the reason.

**GW-R-026** — The `--no-run` note of the `ferrule stage commit` help names the nothing-staged check as applying.

## Adopted commit

**GW-R-030** — `ferrule stage commit <card> --adopt <sha>` logs `commit=<sha>` on the card as a normal stage commit would, creating no commit and leaving the commit's message and history unchanged.

**GW-R-031** — `--adopt` refuses unless the card is `inreview`, the worktree is clean and it is on the run's branch.

**GW-R-032** — `--adopt` refuses unless `<sha>` is the branch's HEAD with exactly one parent.

**GW-R-033** — `--adopt` refuses unless every commit from the branch point with the base branch up to HEAD^ is logged as `commit=` on one of the run's cards.

**GW-R-035** — `--adopt` refuses a commit whose message carries attribution trailers, with a hint to amend the commit if it has not been pushed.

**GW-R-036** — An adopted commit goes through the same push checks, stage checks and review tier as a commit made by `ferrule stage commit`.

**GW-R-037** — `ferrule stage commit` requires `-m` unless `--adopt` is given, and `--adopt` together with `-m` or `--no-run` is a usage error.

**GW-R-038** — The stages guide routes a nothing-staged blocked reason to the blocked stop with its standard options, so answering `a` resumes the stage to adopt the commit.

## Diverged push

**GW-R-007** — `ferrule pr push` refuses with `local=` and `remote=` when the remote branch's head is not an ancestor of the local head, unless a rebase was recorded in the run since its last push.

**GW-R-008** — Only exit code 1 from `git merge-base --is-ancestor`, run after `git fetch`, counts as diverged, never git's error text.

**GW-R-009** — The divergence check runs before the fast checks, so a diverged refusal runs none of them.

**GW-R-010** — A branch that `git ls-remote --exit-code` reports missing from the remote is pushed as a normal first push.

**GW-R-011** — A divergence check that cannot complete for any reason lets the fast checks and the push go on.

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

## Sha routing

**GW-R-082** — The stages guide sends a `reason=sha` push refusal to the sha stop and suggests no re-plan.

**GW-R-083** — The sha stop offers only `a) continue with the next stage, push after its commit`, `b) fixed outside ferrule, push again` and `c) stop`.

**GW-R-084** — The sha stop omits option `a`, keeping the letters `b` and `c`, when every stage card of the run has a `commit=` logged.

**GW-R-085** — The sha stop says the push stays refused until a stage commit lands on top of the current head.

**GW-R-086** — The sha stop names no stage.

**GW-R-087** — `ferrule prompt render blocked` renders the sha stop when given `--mismatch`.

**GW-R-088** — `ferrule prompt render blocked` given `--mismatch` together with `--local` or `--remote` is a usage error.

**GW-R-089** — The sha stop shows the refusal's `mismatch=` reason.

**GW-R-090** — The stages guide sends the sha stop's `1: a` to the first stage in plan order whose card has no `commit=`, with `ferrule pr push` after its commit.

**GW-R-091** — The stages guide sends the sha stop's `1: b` to `ferrule pr push` again.

## Rebased branch

**GW-R-039** — A ferrule command rebases the run's branch onto a named base, moving only the commits from the branch point with the base branch up to HEAD.

**GW-R-040** — The rebase command refuses when the run's worktree has uncommitted changes, untracked files included.

**GW-R-041** — A rebase that hits a conflict is left in progress, and the command's output says to resolve the conflict, stage the files and run `git rebase --continue`, then run the rebase command again.

**GW-R-042** — Once the rebase completes, every commit logged as `commit=` on a stage card of the run between the branch point and the old head is mapped to the rebased commit at the same position, and that commit is appended to the same card as a new `commit=`, in the old order, its old `commit=` lines kept.

**GW-R-043** — The mapping refuses, logging no `commit=`, when the commits from the branch point with the base up to the newest `commit=` logged on the run's cards differ from those up to the rebased HEAD in count, order or commit message.

**GW-R-044** — Only committed stages are mapped; a stage not yet committed commits on the rebased branch as usual.

**GW-R-045** — A mapped rebase makes the last `gauntlet=pass` no longer count, so a gate 3 still ahead runs the gauntlet on the rebased head.

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

**GW-R-062** — In a run with a rebase recorded since its last push, a remote-head check that cannot complete lets the fast checks and the leased force-push go on.

**GW-R-064** — After a mapped rebase, `ferrule run status` prints `gauntlet=-` until a later gauntlet line is logged.
**GW-R-065** — After a mapped rebase, the PR body's status line takes its coverage only from a gauntlet line logged after the run's last `rebase=`.
**GW-R-066** — After a mapped rebase, a stage's auto-continue notice takes its coverage only from a `gauntlet=pass`, and its commit only from a push, each logged after the run's last `rebase=`.

## Push checks

**GW-R-078** — `ferrule pr push` runs only the fast checks before pushing.

**GW-R-079** — The first push after a mapped rebase runs only the fast checks, like every other push.

## Stage done move

**GW-R-101** — `ferrule board move` refuses a stage card `inreview → done` with `reason=unverified` unless the card's last `commit=` is verified.

**GW-R-102** — A stage card's last `commit=` is verified when it equals, or is an ancestor of, a sha logged as `pushed=` or `gauntlet=pass` since the run's last `rebase=`.

**GW-R-103** — Shas logged as `pushed=` or `gauntlet=pass` before the run's last `rebase=` never verify a stage commit.

## PR decisions list

**GW-R-067** — The PR body's decisions list omits a `(chosen)` decision tagged with a local spec ID the run's brief no longer holds.

## Test proof

**GW-R-068** — A ferrule command records each new test's real failure on the run.

**GW-R-069** — For a test command that declares its test files and declaration pattern, `ferrule stage commit` refuses a new test of that command that has no recorded failure, no mutation proof and no mark that it pins existing behaviour under its exact prefixed name.

**GW-R-070** — For a test command that declares its test files and declaration pattern, `ferrule stage commit` lists every existing test or assertion of that command the commit removes or rewrites.

**GW-R-071** — For a test command that declares its test files and declaration pattern, `ferrule stage commit` refuses a removed or rewritten test or assertion of that command that has no named replacement.

**GW-R-072** — A rewritten test may name itself as its replacement and then needs a recorded failure, mutation proof or pins-existing mark, as a new test does.

**GW-R-073** — `ferrule stage red <card> <test> --mutation <text>` records the named test's failure as a mutation proof naming `<text>`.

**GW-R-074** — `ferrule stage pins <card> <test> -- <text>` records the mark that the test pins existing behaviour, with `<text>` naming what it pins.

**GW-R-075** — A recorded failure, mutation proof or pins-existing mark counts only for the stage card it was recorded on.

**GW-R-076** — `ferrule stage commit --replaced <old>=<new>` accepts any test name as the replacement and logs each pair on the card as a replaced-test note.

**GW-R-092** — `ferrule stage commit` logs every `--replaced` pair on the card, whatever the length of its test names.

**GW-R-096** — `ferrule stage red` runs the tests matching `<test>` with the configured test command whose prefix `<test>` starts with, `<test>` without that prefix in place of its placeholder.

**GW-R-098** — `ferrule stage red` with a test command that declares no failure pattern records the named test as failed when the command exits non-zero.

**GW-R-099** — A `ferrule stage red` run that exceeds the test command's timeout is refused with `reason=run`.

**GW-R-100** — The test command's timeout is 30 minutes when `ferrule.toml` sets none.

**GW-R-104** — With a single test command that declares no prefix, `ferrule stage red` passes every test name to it as given.

**GW-R-105** — A `ferrule.toml` configuring two or more test commands is refused when it loads unless every command declares a prefix.

**GW-R-106** — While any test command declares a prefix, `ferrule stage red` refuses a test name that starts with no configured prefix, naming the prefixes, and runs nothing.

**GW-R-107** — `ferrule stage red` with a command that declares a failure pattern records the named test as failed only when the command's output matches the pattern, `{test}` replaced by the name without its prefix.

**GW-R-108** — `ferrule stage commit` finds new, removed and rewritten tests only in the files a test command declares, by that command's declaration pattern, each named with the command's prefix.

**GW-R-109** — In a file of no supported language, a test's body is the lines from its start up to the next declaration in the same file, or the file's end.

**GW-R-110** — `ferrule stage pins` refuses a test name that starts with no configured prefix while prefixes exist, naming the prefixes, and records nothing.

**GW-R-111** — A command's declaration pattern is matched against each test file its `files` globs select, whole, so one match may span lines, with `^` and `$` matching at every line's start and end.

**GW-R-112** — A test starts at the first line of its declaration pattern's match, so lines the pattern matches above the name, such as an attribute or a decorator, are part of the test.

**GW-R-113** — A file's language follows its extension: `.rs` Rust, `.py` Python, `.js` `.mjs` `.cjs` `.jsx` JavaScript, `.ts` `.mts` `.cts` TypeScript, `.tsx` TSX, `.go` Go; any other extension has no supported language.

**GW-R-114** — In a file of a supported language, a test's body node is the innermost syntax node, other than the file's root, that holds the test's name, ends on a line after its declaration's match's last line and starts on or after that match's first line; the test's body ends at that node's last line, or at the match's last line when the test has no body node.

**GW-R-115** — In a file of a supported language, a test with a body node has its body end on the line before the next declaration in the same file when that declaration starts before the body node ends.

**GW-R-116** — In a file of a supported language, when the innermost syntax node, other than the file's root, that holds a test's name and ends on a line after its declaration's match's last line holds a part the parser could not read or had to supply, the test's body is the lines from its start up to the next declaration in the same file, or the file's end, in place of the body GW-R-114 and GW-R-115 give, wherever that node starts.

**GW-R-117** — A test's body holds at least the lines of its declaration's match, even when the next declaration starts on the match's last line.

## Files list

**GW-R-077** — A stage changing a file outside its `files` list is noted on the card and blocks neither the commit nor the review.

## Body checks

**GW-R-080** — PR and issue body assembly refuses prose naming a stage id with `reason=stage-ref`.

**GW-R-081** — The stage-ref check skips a code span or fenced block whose content is anything other than a bare stage id or a bare agent name.

## Ready PR body

**GW-R-093** — Once `ferrule pr publish` has marked the run's PR ready, every later PR body sync of the run writes the full body, never the draft body.

**GW-R-094** — A full body written by a sync after publish reflects the run's state at that sync, as the draft body does before publish.

**GW-R-095** — A PR body sync before `ferrule pr publish` has run writes the draft body.
