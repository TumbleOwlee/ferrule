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
