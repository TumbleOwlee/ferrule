# Session guard

## Format check

**SG-R-001** — The read-only guard lets `cargo fmt --check` run.

**SG-R-002** — The read-only guard lets `cargo fmt` run when `--check` is passed after `--`.

**SG-R-003** — The read-only guard lets `cargo fmt --check` run with `--all` or `-p <pkg>` alongside.

**SG-R-004** — The read-only guard refuses `cargo fmt` whose arguments do not include `--check`.

**SG-R-005** — The format-check allowance applies to every session the read-only guard covers, not only reviews.
