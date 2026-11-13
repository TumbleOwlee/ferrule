# Board — API contract

## Commands

- `ferrule board log <card> <event> [<arg>…] [-- <text>]` — every argument after the first `--` is the line's free text (BO-R-009)

## Stdout

- `card-log=reject reason=<r> event=<e> [<field>=<v>…] hint=<fix>` — the line is not appended; exit 2; `reason=length` for a line over 160 characters (BO-R-008, BO-R-010)

## Stderr

- `ferrule=dev-copy copy=<path> shared=<n> schema=<m>` — printed when a dev build takes its copy; the exit code is unchanged (BO-R-007)
