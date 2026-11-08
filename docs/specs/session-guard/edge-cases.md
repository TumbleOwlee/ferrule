# Session guard — edge cases

## Format check

**SG-E-001** — The read-only guard refuses `cargo fmt --check` when `--print-config` is among its arguments, since rustfmt writes the named file even in check mode.

**SG-E-002** — The read-only guard refuses `cargo fmt --check` when `--config` is among its arguments, also when it follows `--`.

## Commands through rtk

**SG-E-003** — The read-only guard refuses an `rtk` invocation with no subcommand, `rtk --version` included.

**SG-E-004** — Each guard judges an `rtk`-wrapped command inside a pipeline, a command list or a redirect as it judges the bare form there.

**SG-E-005** — The read-only guard refuses an `rtk` invocation carrying, before the subcommand, a flag `rtk` does not declare.

**SG-E-006** — Each guard hands the flags of `rtk read` and their values to the program it judges, so a flag's value counts as an operand of that program.
