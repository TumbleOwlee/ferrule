# Session guard

## Format check

**SG-R-001** — The read-only guard lets `cargo fmt --check` run.

**SG-R-002** — The read-only guard lets `cargo fmt` run when `--check` is passed after `--`.

**SG-R-003** — The read-only guard lets `cargo fmt --check` run with `--all` or `-p <pkg>` alongside.

**SG-R-004** — The read-only guard refuses `cargo fmt` whose arguments do not include `--check`.

**SG-R-005** — The format-check allowance applies to every session the read-only guard covers, not only reviews.

## Rebase in progress

**SG-R-006** — While a rebase is in progress in a run's worktree, the session guard lets through `git add`, `git rm`, `git rebase --continue` and `git rebase --abort` in that worktree.

**SG-R-007** — While a rebase is in progress in a run's worktree, the session guard refuses every git write in that worktree other than those of SG-R-006, `git commit`, `git push` and every other `git rebase` included, with a hint naming what still runs and the rebase command.

**SG-R-008** — Outside a run's worktree with a rebase in progress, the session guard refuses a raw `git rebase`, `--continue` and `--abort` included, with a hint naming the rebase command.

## Commands through rtk

**SG-R-009** — The read-only guard gives `rtk <program> <args>` the verdict it gives `<program> <args>` when `<program>` is `git`, `cargo`, `grep`, `rg`, `find`, `ls`, `tree`, `wc` or `diff`, so SG-R-001 to SG-R-005, SG-E-001 and SG-E-002 hold through `rtk`, except where SG-R-017 refuses.

**SG-R-010** — The session guard gives `rtk <program> <args>` the verdict and refusal hint it gives `<program> <args>` when `<program>` is `git`, `cargo`, `grep`, `rg`, `find`, `ls`, `tree`, `wc` or `diff`, so SG-R-006 to SG-R-008 hold through `rtk`.

**SG-R-011** — The read-only guard judges `rtk read <args>` as `cat <args>` when, before any `--`, `<args>` holds none of `-m`, `--max-lines` and `--tail-lines`, value attached or not.

**SG-R-012** — The session guard judges `rtk read <args>` as `cat <args>` when, before any `--`, `<args>` holds none of `-m`, `--max-lines` and `--tail-lines`, value attached or not.

**SG-R-013** — `rtk`'s global flags before the subcommand do not change the verdict of either guard.

**SG-R-014** — The read-only guard refuses every `rtk` subcommand other than `read`, `bypass rg` and those SG-R-009 lists, `run`, `proxy`, `err`, `test`, `summary`, `curl`, `wget`, `docker` and `config` included.

**SG-R-015** — Each guard judges `rtk read <args>` as `head <args>` when, before any `--`, `<args>` holds `-m` or `--max-lines`, value attached or not.

**SG-R-016** — Each guard judges `rtk read <args>` as `tail <args>` when, before any `--`, `<args>` holds `--tail-lines`, value attached or not, and neither `-m` nor `--max-lines`.

**SG-R-017** — The read-only guard also refuses `rtk grep <args>` wherever it refuses `rg <args>`, since `rtk` runs `rg` for it.

**SG-R-021** — The read-only guard gives `rtk bypass rg <args>` the verdict it gives `rg <args>`, so SG-R-017 holds through `bypass`.

## Read limits

**SG-R-018** — Each guard lets `cat` run on input of at most 200 lines and 1,500 words; SG-R-011 and SG-R-012 carry the limit to `rtk read`.

**SG-R-019** — The limits of SG-R-018 are configurable in the `[guard]` table of ferrule's project configuration.

## Read-only commands

**SG-R-020** — The read-only guard lets `date` and `sha256sum` run.

**SG-R-022** — The read-only guard lets ferrule's read commands run: `board show|list|field|events|vocab`, `run status|diff|log|plan-diff`, `stage verify`, `file headings|section`, `spec id|related|next|refs|collisions|render`, `issue view|search|request|body|milestones`, `pr view|body`, `guide show|list|check`, `prompt lines` and `ci failed`.

**SG-R-023** — The reviewer's read-only guard lets a command append to the run's `review.md`.

## Files written in a call

**SG-R-024** — The session guard refuses a command for running a file written in the same call only when that call also runs the file.

## gh commands

**SG-R-025** — Each guard lets a `gh api graphql` read run.

**SG-R-026** — Each guard lets `gh <args> --help` run when the `gh` command is not one ferrule replaces.

## Settings files

**SG-R-027** — Each guard does not count reading a settings file with `jq` or another of its reading programs as a write.

## Live check

**SG-R-028** — During a live check, the session guard lets `gh api` writes run against the repository named at gate 2.
