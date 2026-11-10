# Git writes — edge cases

## Test proof

**GW-E-015** — An adopted commit gets the new-test and removed-test checks of a normal stage commit.

**GW-E-016** — `ferrule stage commit --no-run` applies neither the new-test check nor the removed-test check.

**GW-E-017** — On a project whose stack is not Rust, `ferrule stage commit` applies neither the new-test check nor the removed-test check and logs a note on the card.

**GW-E-018** — On a Rust project, a test moved to another module or file with its body unchanged counts as neither new nor removed.

**GW-E-023** — On a Rust project, an existing test counts as rewritten only when its body loses or changes a line, whitespace ignored; a test that only gains lines is not rewritten.

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

**GW-E-013** — A rebase whose conflict resolution changed a stage's content is accepted when the commit messages match, and no review sees the resolution; only the gauntlet checks it.

**GW-E-014** — A rebase that rewords a stage commit's message is refused, even when the content is unchanged.

**GW-E-019** — On a Rust project, a test that does not compile gets no recorded failure from `ferrule stage red`.

**GW-E-020** — On a Rust project, macro-generated tests and doc tests are seen by neither the new-test check nor the removed-test check.

**GW-E-021** — On a Rust project, a change to a function outside a test's body counts as rewriting no test.

**GW-E-022** — On a Rust project, tests are told apart by function name, so two tests of one name in different modules count as one test.
