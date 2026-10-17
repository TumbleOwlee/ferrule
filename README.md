# ferrule

One binary for a spec-driven agent workflow: board, cards, specs, issues, PRs, gates and hooks as subcommands, so the workflow's rules live in typed code with tests instead of shell scripts.

## Status

Under construction. Implemented so far:

| Command | Does |
|---|---|
| `ferrule spec sections <file>…` | List a markdown file's headings, fenced code skipped |
| `ferrule spec section [--prefix] <heading>… <file>` | Print the named sections |
| `ferrule spec id [--dir <dir>] <ID>…` | Find spec entries by fixed ID as `file:line:text` |
| `ferrule spec related <keyword>…` | Spec entries mentioning a keyword |
| `ferrule spec collisions <worktree> [<ref>]` | Fixed IDs both this branch and the target ref define |
| `ferrule spec render <spec-diff> <part> [--only <ids>] [--first 0\|1] [--issue-ids <board>]` | Render scenarios, requirements, entries, changes or deferred points of a spec diff |
| `ferrule card field <card> <field>` | A frontmatter field, `log:<key>`, `pass-sha` or `state` |
| `ferrule card log <card> <event> [pos…] [k=v…] [-- text]` | Append one event the vocabulary accepts |
| `ferrule card move <card> <state>` | Move a card, refusing what the board's rules forbid |
| `ferrule card show <card>` / `card board` / `card vocab` | A card as markdown, every card, the event vocabulary |
| `ferrule card new <card> [--state s] [k=v…]` / `card set <card> k=v…` | Create a card, set or clear its fields |
| `ferrule import <board-dir>` | Load a markdown board into `.ferrule/state.db`, all or nothing |
| `ferrule run next <slug> <plan\|replan\|branch\|pr\|ci> <status…>` | The next step after an agent's status line |
| `ferrule run sha-check <slug> s<n> <sha>` | Verify a reported commit before anything is pushed |
| `ferrule run stage-stop <slug> s<n> [--threads n]` | Stop or auto-continue a clean stage, logged on its card |
| `ferrule run reconcile <slug>` | Cards against git before an interrupted run resumes |
| `ferrule run gauntlet <slug> [--force]` | The `ferrule.toml` gauntlet steps in the run's worktree, cached by tree |
| `ferrule run metrics [<slug>]` | A run's metrics line (needs green CI), or every line recorded |
| `ferrule run cleanup <slug>` | Remove worktree and branch, archive cards and artifacts |
| `ferrule run export <slug> [--archive id] [--to dir]` | Write an archive back out as files |
| `ferrule hook pre-tool-use` | Session-wide Bash guard; strips attribution trailers from commit, PR and issue text |
| `ferrule hook reviewer` | Read-only Bash guard for the reviewer agent |

## State

The board lives in `.ferrule/state.db` (`SQLite`) in the main checkout; commands run inside a git worktree reach the same database through `git rev-parse --git-common-dir`. `FERRULE_NOW` overrides the log timestamp. A run's artifacts live in `.ferrule/runs/<slug>/`, its worktree in `.ferrule/worktrees/<slug>/`.

`.ferrule/ferrule.toml` holds what differs between projects; every key is optional:

```toml
main_branch = "main"

[stage_stop]
lines = 400      # a stage diff above this many changed lines stops
files = 8        # or touching more files than this
checkpoint = 3   # stages auto-continued in a row before one stops anyway

[[gauntlet.steps]]
name = "test"
run = ["cargo", "test", "--workspace"]
timeout_secs = 1800

[[gauntlet.steps]]
name = "cov"
run = ["cargo", "llvm-cov", "--workspace", "--fail-under-lines", "80"]
coverage = true  # its TOTAL line gives the pass's cov=
```

## Install

```sh
cargo install --git https://github.com/TumbleOwlee/ferrule --tag v<version> --locked ferrule
```

## Development

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Golden tests compare a command's stdout and exit code with `crates/ferrule/tests/fixtures/expected/<name>.out`. `UPDATE=1 cargo test` rewrites them from the current output; review every rewritten file before committing.
