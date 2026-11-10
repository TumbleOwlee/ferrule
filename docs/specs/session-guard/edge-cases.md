# Session guard — edge cases

## Format check

**SG-E-001** — The read-only guard refuses `cargo fmt --check` when `--print-config` is among its arguments, since rustfmt writes the named file even in check mode.

**SG-E-002** — The read-only guard refuses `cargo fmt --check` when `--config` is among its arguments, also when it follows `--`.

## Commands through rtk

**SG-E-003** — The read-only guard refuses an `rtk` invocation with no subcommand, `rtk --version` included.

**SG-E-004** — Each guard judges an `rtk`-wrapped command inside a pipeline, a command list or a redirect as it judges the bare form there.

**SG-E-005** — The read-only guard refuses an `rtk` invocation carrying, before the subcommand, a flag `rtk` does not declare.

**SG-E-006** — Each guard hands the flags of `rtk read` and their values to the program it judges, so a flag's value counts as an operand of that program.

**SG-E-010** — The read-only guard refuses `rtk bypass <program>` for every program other than `rg`.

## Read limits

**SG-E-007** — Each guard refuses a `cat` over either limit of SG-R-018, its output not captured by a pipe, substitution or redirect, with a hint naming the section read commands.

**SG-E-024** — The limits of SG-R-018 apply to the sum of the lines and the sum of the words of every file a `cat` names.

**SG-E-025** — What `cat` reads from stdin, `-` and a `<` redirect included, does not count against the limits of SG-R-018.

**SG-E-026** — A `cat` operand that is no readable text file, a glob whether it matches or not, a missing path, a directory or a binary file included, adds nothing to the count of SG-R-018.

## Read-only commands

**SG-E-008** — A command that overwrites or truncates `review.md` stays refused by the read-only guard.

**SG-E-014** — The read-only guard refuses `date` with `-s` or `--set`.

**SG-E-021** — The read-only guard lets a ferrule read command of SG-R-022 redirect its output with `>` or `>>` into the system temp directory.

## Files written in a call

**SG-E-020** — `sed` or `awk` counts as running a file written in the same call only when it loads the file as code, through `-f` or, for `awk`, `-E`, `-i`, `-l`, `@include` or `@load`, or when its own script runs commands.

## gh commands

**SG-E-009** — A `gh api graphql` call containing a mutation stays refused.

**SG-E-017** — A `gh` call whose `--help` is not its last word, comes after a word starting with `-`, or has a word holding a shell expansion, gets the verdict it would get without SG-R-026.

**SG-E-018** — A `gh api graphql` call counts as a read only when every `query` field is literal text holding nothing but `query`, `fragment` and anonymous `{` operations at its top level.

**SG-E-019** — A `$name` inside the parentheses of a `gh api graphql` query is taken as a GraphQL variable, so a shell variable placed there is not seen by the guard.

**SG-E-022** — `gh <args> --help` on a `gh` command ferrule replaces stays refused, `gh issue create --help` included.

## Settings files

**SG-E-023** — The session guard refuses a Python read of a settings file, with a hint naming `jq` and `cat`.

## Live check

**SG-E-011** — During a live check, a `gh api` write against any repository other than the named one is refused.

**SG-E-012** — With no repository named at gate 2, `gh api` writes stay refused during a live check.

**SG-E-013** — A live check left open on a run that has since been archived does not open the guard.

**SG-E-015** — During a live check, a `gh api` write whose endpoint does not start with the literal `repos/<owner>/<name>` of the named repository, compared without regard to ASCII case, is refused.

**SG-E-016** — During a live check, a `gh api` write carrying `--hostname`, or with `..`, `%` or a shell expansion in its endpoint, is refused.
**SG-E-027** — During a live check, a call that sets `GH_HOST` — as a prefix on any command, as a bare assignment, through `export`, `declare` or `typeset`, or through `env` — is refused.
