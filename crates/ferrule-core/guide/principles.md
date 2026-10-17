# principles — Rules every run follows

## Principles

Every run, `/spec-request` included.

- Replaces any generic workflow skill (`/workflow`); `docs/specs/` is already the PRD and design record.
- Branch off `{main}`, never commit to `{main}`. `<type>/{slug}`, type ∈ {`feat`, `fix`, `docs`}. **Enforced:** `ferrule hook pre-tool-use` denies `git commit` on `{main}` and `git push` targeting it.
- **Orchestrator never changes spec or code and never reads spec, code, or any agent-written file.** It spawns agents, relays one question at a time between agent and user, and runs ferrule for every mechanical step: `ferrule run init|approve|replan|ci|cleanup`, `ferrule issue file`, `ferrule pr push|publish`, `ferrule card move`, `ferrule run gauntlet`. Every other output is an agent's file (*Agent hand-off* (`ferrule guide handoff`)).
- **Gate 1 is a best-effort grilling, drafted by `spec-author`.** Existing spec + the request's user stories, nothing about current code. It settles scope, scenarios and the edge cases the user cares about; every other point is deferred to the agent that settles it, never anticipated.
- **Issue and PR are the orchestrator's alone**, assembled by ferrule from the run's files (`ferrule issue file`, `ferrule pr push`, `ferrule pr publish`), never retyped. Issue = goal, requirements, scenarios; PR = everything about how; neither names a stage or an agent. No agent but `spec-author` learns an issue or PR number exists — spawn briefs never carry one.
- **An agent's self-reported verification is not verification.** Re-run the tools.
- **A split of an area whose spec costs real context is `spec-author`'s to propose** at gate 1 (rules in `ferrule guide author`) and the user's to approve, never silent.
- **Instructions arrive per step.** `ferrule run next` names the topic for what comes next as `guide: ferrule guide <topic> --slug {slug}`; read it then, not before.

## Principles — implementation

Gate 2 onward.

- **Non-behavior change (`AGENTS.md` `## Workflow`) skips gate 1** and starts at *Architecture* (`ferrule guide architecture`): no `spec-diff.md`; parent card records `gate1: skipped`. Architecture or planning finds a behavior change (`status=behavior-change`) → stop, run gate 1 with the reason, resume the same agent with the `spec-diff.md` path.
- Gate 2 onward delegates to agents. **All agents Sonnet or better.**
- **The draft PR is the user's review surface** (*Every stage* (`ferrule guide stages`); promoted at gate 4, `ferrule guide gate4`). Feedback re-enters the run only through *PR feedback* (`ferrule guide feedback`).
- **One git worktree per run**, `{worktree}` — inside the project directory (agent-reachable), ignored with the rest of `.ferrule/`. Created only by `ferrule run approve gate2` (first thing to touch disk); removed by `ferrule run cleanup` at the end of the run.
- **One implementer at a time**, stages in plan order.
- **A follow-up in the same scope resumes the agent that did the first pass** (`SendMessage`) — reviewer, planner, architect, author — never a respawn; fresh only when that agent is gone (a resumed session) and where a topic says so: every gate 3 `branch` review, a re-plan's planner, each new implementer.
- **The run ends when the PR is ready and CI is green** — then `ferrule run cleanup` removes the worktree and local branch and archives the run. Merge happens outside the workflow.
- **Every state change moves a task-board card.** State living only in conversation dies with the session.
- **Every stop is logged** — `ferrule prompt render` logs `stop kind=<question|approval|stage|blocked>` on the parent card as it renders the stop (`approval`: gate, architecture, epic, delta and re-plan stops). `ferrule run metrics` counts them at the end of the run. A workflow change that adds stops or agents names its expected effect on the metrics in its PR.
