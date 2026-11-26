# Board — edge cases

## Schema migration

**BO-E-004** — A build opening a board whose schema is newer than its own refuses it, a dev build included.

## Dev build copy

**BO-E-001** — A dev build's writes to its copy never reach the shared board.

**BO-E-002** — A kept copy stays in use after the shared board reaches the copy's schema.

**BO-E-003** — A dev build in a repository with no board creates the shared board at its own schema.

**BO-E-005** — A dev build that fails to take its copy leaves no temporary copy file in the checkout.

## Board dump

**BO-E-006** — A dump with no matching run, an empty board or a `--run` slug not on the board, prints `{"version":1,"runs":[]}` and exits 0.

**BO-E-007** — A waiting stop for which ferrule opened no files in the viewer shows `files` as an empty list.

**BO-E-008** — An archived run's `waiting` is `null`.

**BO-E-009** — `--state` matches an archived run by the state its card had when it was archived.

**BO-E-010** — An archive taken when the run's card was already off the board shows `card` as `null` and lists the stage cards it holds.

**BO-E-011** — A waiting stop that opened no files, logged in the same minute as an earlier stop that did, shows the earlier stop's files.

**BO-E-012** — An approval stop without a gate is answered by any later `approved=`.

**BO-E-013** — A run whose stage cards are on the board without its run card shows `card` as `null` and lists those stage cards.

**BO-E-014** — `--state` never matches a run entry whose `card` is `null`.
