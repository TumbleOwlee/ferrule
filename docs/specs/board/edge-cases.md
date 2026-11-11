# Board — edge cases

## Schema migration

**BO-E-004** — A build opening a board whose schema is newer than its own refuses it, a dev build included.

## Dev build copy

**BO-E-001** — A dev build's writes to its copy never reach the shared board.

**BO-E-002** — A kept copy stays in use after the shared board reaches the copy's schema.

**BO-E-003** — A dev build in a repository with no board creates the shared board at its own schema.
