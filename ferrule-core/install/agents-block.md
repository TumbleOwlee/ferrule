## Workflow — ferrule

Behavior changes go through the spec-driven workflow: `/fe-feature` (a request only: `/fe-request`; an outside review: `/fe-review`). Instructions arrive per step: `ferrule run next` names the topic that applies now, `ferrule guide --list` every topic.

- Git and GitHub writes are ferrule's: `ferrule card commit`, `ferrule pr push`, `ferrule issue file`, `ferrule pr publish`; the session guard names the replacement for any raw call. Outside a run: `ferrule card commit --no-run -m <msg>`, `ferrule pr push --no-run`.
- Read long markdown by section: `ferrule spec sections <file>`, then `ferrule spec section '<heading>' <file>`; a spec entry by ID: `ferrule spec id <ID>`.
- Read issues and PRs with `ferrule issue view <n>` and `ferrule pr view <n>`.
- The board lives in `.ferrule/state.db`: `ferrule card board`, `ferrule card show <id>`.
