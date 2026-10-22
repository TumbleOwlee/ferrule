<!-- ferrule:begin -->
## Workflow — ferrule

Behavior changes go through the spec-driven workflow: `/fe-feature` (a request only: `/fe-request`; an outside review: `/fe-review`). Instructions arrive per step: `ferrule gate next` names the topic that applies now, `ferrule guide list` every topic.

- Git and GitHub writes are ferrule's: `ferrule stage commit`, `ferrule pr push`, `ferrule issue post`, `ferrule pr publish`; the session guard names the replacement for any raw call. Outside a run: `ferrule stage commit --no-run -m <msg>`, `ferrule pr push --no-run`.
- Read long markdown by section: `ferrule file headings <file>`, then `ferrule file section '<heading>' <file>`; a spec entry by ID: `ferrule spec id <ID>`.
- Read issues and PRs with `ferrule issue view <n>` and `ferrule pr view <n>`: the body, then one line per comment; `--comment <n|a-b>` prints those comments in full, `--comments` all of them.
- The board lives in `.ferrule/state.db`: `ferrule board list`, `ferrule board show <id>`.
<!-- ferrule:end -->
