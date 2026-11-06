# Git writes — edge cases

## Known gaps

**GW-E-001** — A remote that moves after the divergence check passes fails at the push with the existing `reason=push` refusal.

**GW-E-002** — After a merge outside ferrule resolves a divergence, `ferrule pr push` in a run refuses with `reason=sha` until the next `ferrule stage commit` lands on top of it.

**GW-E-003** — No command adopts a commit made outside ferrule as a stage's `commit=`, so a stage whose changes were committed that way stays blocked.

**GW-E-004** — A merge in progress whose index equals HEAD gets no nothing-staged refusal; `git commit` concludes it as before.

**GW-E-005** — A run in the state GW-E-002 describes with no further change to commit stays refused, because `ferrule stage commit` needs staged changes.

**GW-E-006** — A retry after the failed push of GW-E-001 gets the diverged refusal.

**GW-E-007** — A helper process that git leaves running after a divergence check finished within the 60-second limit keeps running.

**GW-E-008** — On Windows, a helper whose parent git had already exited when a divergence check reached its 60-second limit keeps running, contrary to GW-R-029.

**GW-E-009** — On targets that are neither unix nor Windows, every process a divergence check started keeps running after the check reaches its 60-second limit, contrary to GW-R-029.

**GW-E-010** — A process that left git's process group, such as a daemon that calls setsid, keeps running after a divergence check reaches its 60-second limit, contrary to GW-R-029.
