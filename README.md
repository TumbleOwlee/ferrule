# ferrule

One binary for a spec-driven agent workflow: board, cards, specs, issues, PRs, gates and hooks as subcommands, so the workflow's rules live in typed code with tests instead of shell scripts.

## Status

Under construction. Implemented so far:

| Command | Does |
|---|---|
| `ferrule setup install [--stack s] [--force]` / `setup check` | Set up or upgrade `.ferrule/` and the Claude Code adapters; `check` fails on drift |
| `ferrule setup uninstall [--purge]` | Remove what `setup install` added |
| `ferrule guide show <topic> [--section <heading>…] [--slug <s>] [--stage s<n>] [--lane fast\|full]` | One topic of the workflow's instructions, or some of its sections, cut to the run's lane, steps the card shows done marked, placeholders filled; `guide list` groups orchestrator and agent topics, `guide check` |
| `ferrule run init <slug> [--issue <n>]` | Start a run: parent card, run directory, the named issue's request and brief; a slice's epic |
| `ferrule run reconcile <slug> [--local]` | Cards against git, and the issue, PR and PR body on the host, before an interrupted run resumes |
| `ferrule run rebuild <issue> <pr>` | Recreate a lost board from the issue, the draft PR and git; re-adds the worktree |
| `ferrule run cleanup <slug>` | Remove worktree and branch, archive cards and artifacts |
| `ferrule run metrics [<slug>]` | A run's metrics line (needs green CI), or every line recorded |
| `ferrule run export <slug> [--archive id] [--to dir]` | Write an archive back out as files |
| `ferrule gate approve <slug> <gate> [--<flag>…]` | Record an approval; gate 2 files the issue, adds the worktree, creates the stage cards from `plan.md`'s `## Shared` table |
| `ferrule gate next <slug> <gate2\|replan\|gate3\|gate4\|ci> <status…>` | The next step after an agent's status line |
| `ferrule stage replan <slug> start\|approve [--stops s1,…]` | Snapshot the plan and log `from=`; swap the uncommitted stage cards for the new plan's |
| `ferrule stage verify <slug> s<n> <sha>` | Verify a reported commit before anything is pushed |
| `ferrule stage stop <slug> s<n> [--threads n]` | Stop or auto-continue a clean stage, logged on its card |
| `ferrule stage commit <card> -m <msg>` / `--no-run -m <msg>` | Commit for a stage in review: never on the main branch, trailers stripped, `commit=` logged |
| `ferrule ci wait <slug>` | Wait for the PR's checks, log `ci=`, print the next step and the failed run's errors |
| `ferrule ci gauntlet <slug> [--force]` | The `ferrule.toml` gauntlet steps in the run's worktree, cached by tree |
| `ferrule ci failed [<branch>] [--run <id>] [--job <name>] [--context <n>]` | The error lines of a failed CI run, the branch's latest unless `--run` names one, each with `n` lines after it |
| `ferrule board field <card> <field>` | A frontmatter field, `log:<key>`, `pass-sha` or `state` |
| `ferrule board log <card> <event> [pos…] [k=v…] [-- text]` | Append one event the vocabulary accepts |
| `ferrule board move <card> <state>` | Move a card, refusing what the board's rules forbid; re-renders the run's draft PR body |
| `ferrule board show <card>` / `board list` / `board vocab` | A card as markdown, every card, the event vocabulary |
| `ferrule board new <card> [--state s] [k=v…]` / `board set <card> k=v…` | Create a card, set or clear its fields |
| `ferrule board import <board-dir>` | Load a markdown board into `.ferrule/state.db`, all or nothing |
| `ferrule spec id [--dir <dir>] <ID>…` | Find spec entries by fixed ID as `file:line:text` |
| `ferrule spec related <keyword>…` | Spec entries mentioning a keyword |
| `ferrule spec collisions <worktree> [<ref>]` | Fixed IDs both this branch and the target ref (default `origin/<main_branch>`) define |
| `ferrule spec render <spec-diff> <part> [--only <ids>] [--with-unnumbered] [--issue-ids]` | Render scenarios, requirements, entries, changes or deferred points of a spec diff |
| `ferrule file headings <file>…` | List a markdown file's headings, fenced code skipped |
| `ferrule file section [--prefix] <heading>… <file>` | Print sections by heading text (`Status`, any case; `## Status` pins the level) |
| `ferrule prompt lines [--seen n] <file>…` | The lines of approval files a prompt quotes, then `Confirm:` |
| `ferrule prompt show [--no-lines] <file>…` | Open files in the `[viewer]` (else stdout) and record the run's approval marker; exit 3 for a summary over its cap |
| `ferrule prompt render <slug> <kind> [--<flag>…]` | A stop's whole message (approval, question, notice) or the `approved` record; logs the stop |
| `ferrule issue view <n\|url\|KEY>` | An issue's title, body and comments as compact text |
| `ferrule issue request <n> [--spec\|--epic\|--slice-line\|--comments]` | A reused issue's original request, or the part named |
| `ferrule issue body <slug> [--slice <n>]` | The tracking issue's body from the run's artifacts; exit 3 when it names a stage or an agent |
| `ferrule issue post <slug> [--slice <n>\|--split\|--follow-up <k>\|--comment] [--reuse <n>]` | Post an issue or comment from the run's files; adopts an open issue with the same title |
| `ferrule pr view <n>` | A PR's title, body and comments as compact text |
| `ferrule pr body <slug> draft\|full` | The PR body from the run's artifacts and cards; exit 3 when it names a stage or an agent |
| `ferrule pr open <slug>` | Open the draft PR from the rendered draft body |
| `ferrule pr push <slug>` | Push a verified, gauntlet-green head; update the draft body (the first push opens the PR) |
| `ferrule pr push --no-run` | Push the current branch outside a run, never the main one |
| `ferrule pr publish <slug> [--follow-ups 1,3\|none]` | Gate 4: threads, ID collisions, full body, ready, follow-ups filed |
| `ferrule pr feedback route <slug>` | Fetch the threads, log `feedback=`, move the stages owning the commented files back |
| `ferrule pr feedback fetch <n> <out>` / `pr feedback reply <n> <file> [<stage> <sha>]` | Unresolved review threads into a file; post the filled replies, resolving fixed threads |
| `ferrule hook shell` | Session-wide Bash guard: refuses writes to `.ferrule/state.db` and the lock, output floods, and the raw git and `gh` calls ferrule runs itself, naming the command to run instead |
| `ferrule hook attribution` | Hands the Bash command back with attribution trailers removed |
| `ferrule hook reviewer` | Read-only Bash guard for the reviewer agent |
| `ferrule hook session-start` | Names the cards an interrupted run left in progress |
| `ferrule hook git pre-commit\|commit-msg\|pre-push` | Git hooks: no commit or push to the main branch; trailers stripped from commit messages |

The names before the topic split still run for one release, hidden from help: `card …` (now `board`, with `card commit` now `stage commit`), `install`/`uninstall` (`setup`), `guide <topic>` and `--list`/`--check` (`guide show|list|check`), `run approve|next` (`gate`), `run sha-check|stage-stop|replan` (`stage verify|stop|replan`), `run gauntlet|ci` and `pr failed` (`ci gauntlet|wait|failed`), `spec section|sections` (`file section|headings`) and `issue file` (`issue post`).

## Output

Commands print `key=value` status lines; exit codes are 0 done, 1 refused, 2 usage, 3 content. Commands that print a document (`guide show`, `file section`, `file headings`, `spec render`, `board show`, `board list`, `issue view|request|body`, `pr view|body`) style it when stdout is a terminal: headings indent their sections, `file headings` draws a tree, lists and tables are aligned and wrapped to the terminal's width, and a table too wide becomes one record per row. Piped or redirected, which is what agents and scripts get, they print markdown unchanged. `--format text|md` overrides the choice; `NO_COLOR` keeps the layout without colours. A refusal's status line ends in `hint=<fix>` (always the last key, free text to the end of the line) unless it already names its fix. `-h` and `--help` print the same full help, with `Output:`, `Refusals:` and `See: ferrule guide show <topic>` sections for each command. A command other than `setup` refuses on stderr with `ferrule=refused reason=version …` when the binary and the project's version differ. `prompt` output is already shaped for the user and prints as is.

## Guide

The workflow's instructions ship inside the binary as topics (`crates/ferrule-core/guide/<topic>.md`): one per orchestrator step (`gate2`, `stages`, `gate4`, …) and one per agent (`implementer`, `planner`, …). `ferrule gate next` names the topic for what comes next as a `guide:` line.

- `<!-- lane: fast -->` (or `full`) on the line above a block keeps it for that lane only; above a heading it covers the heading's section. Without a lane both are printed, labelled.
- `<!-- done: <token> -->` marks a block `(done)` once the run's parent card carries the token (`pr=ready`, or a bare field name such as `gate2`).
- `{slug}`, `{stage}`, `{card}`, `{run_dir}`, `{worktree}`, `{branch}`, `{issue}`, `{pr}`, `{main}` are filled from the run; unknown ones print as `<slug>`.
- A project overrides a topic with `.ferrule/guide/<topic>.md`. Without headings the file replaces the topic; with headings each of its sections replaces the default's section of the same heading, and a section the default lacks is appended. A file naming no shipped topic is a topic of the project's own; its first line is `# <topic> — <summary>`.
- `ferrule guide check` fails when a topic names a topic or a `--section` that does not exist, or carries a malformed tag.
- A topic points at part of another as `ferrule guide show <topic> --section '<heading>'`.

`plan.md`'s `## Shared` section opens with the stage table `ferrule gate approve <slug> gate2` creates the stage cards from (`ferrule guide show planner` defines it):

```
| Stage | Title | Files | Blocked by |
|---|---|---|---|
| s1 | Add backoff | src/client.rs, tests/client.rs | s0 |
```

## State

The board lives in `.ferrule/state.db` (`SQLite`) in the main checkout; commands run inside a git worktree reach the same database through `git rev-parse --git-common-dir`. `FERRULE_NOW` overrides the log timestamp. A run's artifacts live in `.ferrule/runs/<slug>/`, its worktree in `.ferrule/worktrees/<slug>/`.

`.ferrule/ferrule.toml` holds what differs between projects; every key is optional:

```toml
main_branch = "main"

[backend]
tracker = "github"           # github (gh) | jira (.ferrule/jira.local.json: baseUrl, email, apiToken)
ci = "github"                # github (gh) | bitbucket (.ferrule/bitbucket.local.json: workspace, repoSlug, username, appPassword)

[[gauntlet.step]]
name = "test"
cmd = ["cargo", "test", "--workspace"]
timeout = "30m"

[[gauntlet.step]]
name = "cov"
cmd = ["cargo", "llvm-cov", "--workspace", "--fail-under-lines", "80"]
coverage = "llvm-cov"        # how the pass's cov= is read from the output

[viewer]
command = ["glow"]           # prompt show appends the file path; unset: stdout

[hook.readonly]
mutating = ["cargo add"]     # word prefixes the reviewer's guard also refuses

[thresholds]
read_budget = 3              # read events one stage card may carry
stage_stop_lines = 400       # a stage diff above this many changed lines stops
stage_stop_files = 8         # or touching more files than this
stage_stop_checkpoint = 3    # stages auto-continued in a row before one stops anyway
summary_max_lines = 25       # caps on *.summary.md and *.verdict.md
summary_max_bytes = 2048
related_max = 20             # spec related refuses more hits than this
```

## Install

```sh
cargo install --git https://github.com/TumbleOwlee/ferrule --tag v<version> --locked ferrule
cd <repository> && ferrule setup install
```

`ferrule setup install` writes:

| Path | Committed | What |
|---|---|---|
| `.ferrule/ferrule.toml` | yes | Created once (stack detected, a gauntlet for it, `glow` as viewer when on `PATH`); later runs only move `version` |
| `.ferrule/ferrule.lock` | yes | The installed version and the hash of every generated file and block |
| `.ferrule/ferrule` | yes | Shim the hooks call: runs `ferrule`, or refuses with the install command when it is missing |
| `.ferrule/.gitignore` | yes | Ignores the board, run files and worktrees |
| `.ferrule/lefthook.yml` | yes | The git hooks, when the project uses lefthook (an `extends` block is added to its config) |
| `.claude/agents/fe-{author,architect,planner,implementer,reviewer}.md` | yes | Identity, status line, never-do rules, the first `ferrule guide` topic |
| `.claude/skills/fe-{feature,request,review,bootstrap}/SKILL.md` | yes | The entry points |
| `.claude/settings.json` | yes | The hooks and the `permissions.deny` backstop, merged into the file |
| `AGENTS.md` | yes | A block between `<!-- ferrule:begin -->` and `<!-- ferrule:end -->` |
| `CLAUDE.md` | yes | An `@AGENTS.md` line when it has none |
| `docs/specs/README.md` | yes | The ID and numbering rules, when the file is missing |
| `.git/hooks/{pre-commit,commit-msg,pre-push}` | no | Without lefthook or husky |

Re-running `ferrule setup install` upgrades: a generated file whose hash matches the lock is replaced, an edited one is kept with the new version next to it as `<name>.ferrule-new`. A file at a generated path that ferrule never wrote is refused unless `--force`. A `version` in `ferrule.toml` other than the binary's refuses every command but `setup`.

A binary whose version differs from `version` in `ferrule.toml` or in `ferrule.lock` refuses every command but `setup`, and names the fix: an older binary than the project's says which tag to `cargo install`; a newer one says to run `ferrule setup install` and commit the updated files. The Bash guard refuses every call the same way except those two updates, and the session-start hook reports the mismatch.

The session guard denies the raw git and `gh` calls ferrule replaces; `[guard] allow = ["gh api"]` in `ferrule.toml` re-opens one, in the guard and the deny rules alike. Two are denied only in some forms, by the guard alone since a deny rule matches a prefix: `gh api` when it writes, `gh run view` with `--log` or `--log-failed` (`ferrule ci failed` replaces those).

## Development

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Golden tests compare a command's stdout and exit code with `crates/ferrule/tests/fixtures/expected/<name>.out`. `UPDATE=1 cargo test` rewrites them from the current output; review every rewritten file before committing.
