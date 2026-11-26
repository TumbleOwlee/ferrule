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

## Card log

**BO-R-008** — A `ferrule board log` call that answers `card-log=reject`, for any reason, leaves the card's log unchanged.

**BO-R-009** — `ferrule board log` takes every argument after the first `--` as the line's free text, words starting with `--` included.

**BO-R-010** — A card log line longer than 160 characters, timestamp included, is refused with `reason=length`, a `test-replaced` line excepted.

## Board dump

**BO-R-011** — `ferrule board dump` prints one JSON document holding `version` and `runs`, one entry per run on the board.

**BO-R-012** — A run entry holds its run card's id, state and fields as an object in stored order.

**BO-R-013** — A run entry holds the events of its parent and stage cards, oldest first, each with its card, time, name without a valued event's trailing `=`, value, free text, keyed arguments as an object where a repeated key keeps its last value, and positional words as a list.

**BO-R-014** — A run entry's `waiting` is the run's last `stop` event when no later event answers it, and `null` otherwise.

**BO-R-015** — An approval stop is answered by a later `approved=` whose value equals the stop's `gate`, whatever the gate's name, the retired `delta` gate that older cards still carry included (GA-E-024).

**BO-R-016** — A stop whose kind is not `approval`, a stop without a kind included, is answered by any later event on one of the run's cards.

**BO-R-017** — `waiting` carries the stop's kind, gate, time and the absolute paths of the files ferrule opened in the viewer for that stop, in the order it opened them.

**BO-R-018** — The document's `version` is 1 and changes only when the document changes in a way that breaks an existing consumer.

**BO-R-019** — `--run <slug>` limits the document to the entries whose slug is `<slug>`, its archived entries included when `--archived` is passed.

**BO-R-020** — Archived runs appear in the document only when `--archived` is passed.

**BO-R-021** — `ferrule board dump` leaves the board unchanged.

**BO-R-022** — A run entry's `archived` is `null` for a run on the board and for an archived run holds the archive's time and whether the run was abandoned.

**BO-R-023** — `ferrule board dump` prints JSON whatever the global `--format` option says.

**BO-R-024** — Each archive is its own run entry under the run's slug, after the runs on the board, so a slug archived twice, or both on the board and archived, has one entry for each.

**BO-R-025** — `--state <s>`, repeatable, `<s>` one of `open`, `inprogress`, `inreview` and `done`, limits the document to runs whose card is in one of the states, each with all its stage cards.

**BO-R-026** — A run entry holds its stage cards in stage order, each with its id, its stage as `s<n>`, its state and its fields as an object in stored order.

**BO-R-027** — Run entries of runs on the board appear in slug order.

**BO-R-028** — Archive entries appear oldest archive first.
