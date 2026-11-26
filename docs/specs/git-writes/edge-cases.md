# Git writes — edge cases

## Test proof

**GW-E-015** — An adopted commit gets the new-test and removed-test checks of a normal stage commit.

**GW-E-016** — `ferrule stage commit --no-run` applies neither the new-test check nor the removed-test check.

**GW-E-017** — For a test command that declares no test files or declaration pattern, `ferrule stage commit` applies neither the new-test check nor the removed-test check to its tests and logs a note on the card.

**GW-E-018** — A test moved to another file of the same command with its body unchanged counts as neither new nor removed.

**GW-E-023** — An existing test counts as rewritten only when its body loses or changes a line, whitespace ignored; a test that only gains lines is not rewritten.

**GW-E-029** — On any project with no configured test command, `ferrule stage red` and `ferrule stage pins` refuse with `reason=no-command`.

**GW-E-031** — A `ferrule.toml` in which any test command has no argument containing `{test}` is refused when it loads.

**GW-E-034** — A test command that declares no failure pattern and exits zero gets `red=refused reason=no-failure` from `ferrule stage red`.

**GW-E-037** — A `ferrule.toml` in which one test command's prefix starts another's is refused when it loads, naming both.

**GW-E-039** — A command with a failure pattern whose output does not match it gets `red=refused reason=no-failure` from `ferrule stage red`, whatever its exit code.

**GW-E-041** — A `ferrule.toml` whose failure or declaration pattern is not a valid regular expression is refused when it loads, naming the command.

**GW-E-042** — A `ferrule.toml` whose declaration pattern has no group named `name` is refused when it loads.

**GW-E-043** — A `ferrule.toml` in which a test command declares test files without a declaration pattern, or a pattern without files, is refused when it loads.

**GW-E-044** — With no configured test command, `ferrule stage commit` applies neither the new-test check nor the removed-test check and logs a note on the card.

## Known gaps

**GW-E-001** — A remote that moves after the divergence check passes fails at the push with the existing `reason=push` refusal.

**GW-E-002** — After a merge outside ferrule resolves a divergence, `ferrule pr push` in a run refuses with `reason=sha` until the next `ferrule stage commit` lands on top of it.

**GW-E-003** — No command adopts a merge commit made outside ferrule as a stage's `commit=`, so a stage whose changes were committed that way stays blocked.

**GW-E-004** — A merge in progress whose index equals HEAD gets no nothing-staged refusal; `git commit` concludes it as before.

**GW-E-005** — A run in the state GW-E-002 describes with no further change to commit stays refused, because `ferrule stage commit` needs staged changes.

**GW-E-006** — A retry after the failed push of GW-E-001 gets the diverged refusal.

**GW-E-007** — A helper process that git leaves running after a divergence check finished within the 60-second limit keeps running.

**GW-E-008** — On Windows, a helper whose parent git had already exited when a divergence check reached its 60-second limit keeps running, contrary to GW-R-029.

**GW-E-009** — On targets that are neither unix nor Windows, every process a divergence check started keeps running after the check reaches its 60-second limit, contrary to GW-R-029.

**GW-E-010** — A process that left git's process group, such as a daemon that calls setsid, keeps running after a divergence check reaches its 60-second limit, contrary to GW-R-029.

**GW-E-011** — `--adopt` refuses a HEAD already logged as `commit=` on any of the run's cards.

**GW-E-013** — A rebase whose conflict resolution changed a stage's content is accepted when the commit messages match, and no review sees the resolution; only the fast checks, CI and a gate 3 still ahead check it.

**GW-E-014** — A rebase that rewords a stage commit's message is refused, even when the content is unchanged.

**GW-E-020** — A test whose declaration its command's declaration pattern does not match is seen by neither the new-test check nor the removed-test check.

**GW-E-022** — Tests are told apart by their prefixed name as the declaration pattern captures it, so two tests of one name in different files of one command count as one test.

**GW-E-025** — With no stage left to commit after a merge outside ferrule, the sha stop's push again returns the same refusal to the same stop; only stop ends it (GW-E-002, GW-E-005).

**GW-E-030** — A non-zero exit of a test command that declares no failure pattern is recorded as the named test's red whatever its cause, a failure of the environment it needs included.

**GW-E-033** — With a configured test command, a test marked ignored runs only when that command's own arguments include ignored tests; ferrule adds none.

**GW-E-035** — For a test command that declares no test detection, the `test-red` it records is read by no check: `ferrule stage commit` applies no test check (GW-E-017) and the stage checks run no new tests (GA-E-010); it stays on the card as the proof a reviewer reads.

**GW-E-036** — Done-move verification checks ancestry against the logged sha in the local repository, not the remote branch; a remote changed outside ferrule after the push is not noticed.

**GW-E-038** — A helper or fixture between two tests in one file of no supported language counts as part of the earlier test's body, so changing it rewrites that test.

**GW-E-040** — A `pushed=` or `gauntlet=pass` sha the local repository does not hold verifies only a stage commit equal to it, never one it would contain.

**GW-E-045** — Lines of an enclosing test after a nested test's declaration belong to no test's body.

## Body checks

**GW-E-024** — A code span or fenced block whose whole content is a bare stage id or a bare agent name is refused with `reason=stage-ref`.

## Ready PR body

**GW-E-026** — A published PR whose run reopens stages through the PR feedback route or a replan keeps the full body; it is never returned to the draft body.

**GW-E-027** — A PR marked ready outside ferrule before `ferrule pr publish` has run keeps receiving the draft body from every body sync.

**GW-E-028** — A PR returned to draft outside ferrule after `ferrule pr publish` has run keeps receiving the full body from every body sync.
