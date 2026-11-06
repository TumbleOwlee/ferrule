# Git writes — edge cases

## Known gaps

**GW-E-001** — A remote that moves after the divergence check passes fails at the push with the existing `reason=push` refusal.

**GW-E-002** — After a merge outside ferrule resolves a divergence, `ferrule pr push` in a run refuses with `reason=sha` until the next `ferrule stage commit` lands on top of it.

**GW-E-003** — No command adopts a commit made outside ferrule as a stage's `commit=`, so a stage whose changes were committed that way stays blocked.

**GW-E-004** — A merge in progress whose index equals HEAD gets no nothing-staged refusal; `git commit` concludes it as before.

**GW-E-005** — A run in the state GW-E-002 describes with no further change to commit stays refused, because `ferrule stage commit` needs staged changes.

**GW-E-006** — A retry after the failed push of GW-E-001 gets the diverged refusal.
