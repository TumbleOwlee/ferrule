# Board

## Schema migration

**BO-R-001** — A release build opening a board whose schema is older than its own migrates the board in place.

**BO-R-002** — A dev build, any build with debug assertions on, never migrates the shared board.

## Dev build copy

**BO-R-003** — A dev build opening a shared board whose schema is older than its own works on its own copy of the board, migrated to its schema.

**BO-R-004** — A dev build opening a shared board whose schema equals its own works on the shared board.

**BO-R-005** — A dev build's copy is taken once per worktree, the main checkout counting as one, and later dev-build commands there use it.

**BO-R-006** — A dev build's copy is removed when its worktree is removed.

**BO-R-007** — A dev build taking its copy from a shared board of an older schema prints one stderr line naming the copy and both schemas.
