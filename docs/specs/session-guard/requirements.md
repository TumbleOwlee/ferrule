# Session guard

## Format check

**SG-R-001** — The read-only guard lets `cargo fmt --check` run.

**SG-R-002** — The read-only guard lets `cargo fmt` run when `--check` is passed after `--`.

**SG-R-003** — The read-only guard lets `cargo fmt --check` run with `--all` or `-p <pkg>` alongside.

**SG-R-004** — The read-only guard refuses `cargo fmt` whose arguments do not include `--check`.

**SG-R-005** — The format-check allowance applies to every session the read-only guard covers, not only reviews.

## Rebase in progress

**SG-R-006** — While a rebase is in progress in a run's worktree, the session guard lets through `git add`, `git rm`, `git rebase --continue` and `git rebase --abort` in that worktree.

**SG-R-007** — While a rebase is in progress in a run's worktree, the session guard refuses every git write in that worktree other than those of SG-R-006, `git commit` and `git push` included.

**SG-R-008** — The session guard refuses a raw `git rebase` with a hint naming the rebase command, except `--continue` and `--abort` while a rebase is in progress in a run's worktree.
