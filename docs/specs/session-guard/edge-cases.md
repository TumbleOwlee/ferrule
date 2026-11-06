# Session guard — edge cases

## Format check

**SG-E-001** — The read-only guard refuses `cargo fmt --check` when `--print-config` is among its arguments, since rustfmt writes the named file even in check mode.

**SG-E-002** — The read-only guard refuses `cargo fmt --check` when `--config` is among its arguments, also when it follows `--`.
