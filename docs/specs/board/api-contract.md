# Board — API contract

## Commands

- `ferrule board log <card> <event> [<arg>…] [-- <text>]` — every argument after the first `--` is the line's free text (BO-R-009)
- `ferrule board dump [--run <slug>] [--state <s>…] [--archived]` — read-only JSON dump of the board, printed compact on one line whatever `--format` says (BO-R-011, BO-R-019, BO-R-020, BO-R-023, BO-R-025)

## Stdout

- `card-log=reject reason=<r> event=<e> [<field>=<v>…] hint=<fix>` — the line is not appended; exit 2; `reason=length` for a line over 160 characters (BO-R-008, BO-R-010)
- the dump document: `version`, `runs[]` with `slug`, `card` (`id`, `state`, `fields`), `stages[]` (`id`, `stage`, `state`, `fields`), `events[]` (`card`, `at`, `name`, `value`, `args`, `positional`, `text`), `waiting` (`kind`, `gate`, `at`, `files`) and `archived` (`at`, `abandoned`); times as stored, `YYYY-MM-DDTHH:MM` UTC (BO-R-011..BO-R-017, BO-R-022, BO-R-024, BO-R-026)

## Stderr

- `ferrule=dev-copy copy=<path> shared=<n> schema=<m>` — printed when a dev build takes its copy; the exit code is unchanged (BO-R-007)
