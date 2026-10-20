# board — Cards and run artifacts

## Task board

`.ferrule/state.db` holds the cards (*Cards*), the gauntlet's pass cache, one metrics line per finished run (`ferrule run metrics`) and the archive of every finished run's cards and artifacts (`ferrule run cleanup`; `ferrule run export {slug}` writes one back out as files). Metrics and archive are never cleaned up. Each live run's files sit in `{run_dir}` (*Artifacts*).

### Artifacts

```
{run_dir}/
  spec-diff.md       gate 1 brief, then the run's spec until s0 — local IDs until gate 2; synced from the branch at gate 4
  decisions.md       every decision of the run, one line each, appended by every agent — the one source of Confirm: and the PR's ## Decisions
  architecture.md    components, structure, interactions, public surface, code map — the plan's target
  architecture.summary.md  architecture decisions the user approves, capped
  plan.md            gate 2 stages, steps, dependency tree — the implementer's contract
  plan.summary.md    gate 2 decisions the user approves, capped
  plan.prev.md       plan.md as the last plan review saw it, for the follow-up pass
  review.md          review findings + history, keyed by stage id, append-only
  review.verdict.md  open findings per stage, rewritten every pass, capped
  gauntlet.log       full output of the last ferrule ci gauntlet
  question.md        an agent's batch of decisions for the user (ferrule guide show question)
  slices.md          epic run: the ordered slices, each later its own issue (ferrule guide show epic)
  epic.md            slice run: the epic issue's spec and skeleton architecture (ferrule guide show epic)
  slice-line.md      slice run: the slice issue's **Part of epic** line
  issue.md           title + goal + out of scope (ferrule guide show issue)
  request.md         the request issue's original body, stored once (gate 1 or reuse)
  issue-spec.md      the brief a reused issue already carries
  issue-comment.md   changelog comment of the latest user-approved spec change after filing
  split-issue.md     an issue split off (ferrule guide show split)
  follow-up-<k>.md   gate 4: an issue for something the run noticed outside its goal
  pr.md              gate 4 PR title + body (ferrule guide show gate4)
  pr-feedback.md     unresolved PR review threads (ferrule guide show feedback)
  ci.log             the last ferrule ci wait wait
```

`{run_dir}` is in the **main checkout**: every spawn brief passes its absolute path (an agent in a worktree would resolve a relative one inside the worktree). Every ferrule command finds the main checkout itself, from any worktree.

### Cards

A card's state is one of `open`, `inprogress`, `inreview`, `done`. Every move, the implementer's included, is `ferrule board move <card-id> <state>`: it refuses a transition the table below forbids (a stage to `done` without a `commit=` the parent's `gauntlet=pass` verified, a stage started before its `blocked-by`, the parent before `gate2`), logs `moved=<state>` and, once the run has a PR, re-renders its draft body (`pr-body=…`). "card → `<state>`" in this guide means that call. `ferrule board show <id>` prints a card, `ferrule board list` every card with its state.

Cards live in the main checkout's database only — a worktree agent gets its own card's id and writes only that card (`ferrule guide show implementer` says how).

| Card | Id | Owner |
|---|---|---|
| parent | `{slug}` | orchestrator — one per run |
| stage | `{slug}.s<n>` | the implementer working that stage |

Cards are agent-only: fields + a log of terse field=value tokens, no prose. Log lines are appended by `ferrule board log`, never rewritten.

```
id: {slug}.s3
parent: {slug}
title: <title>
blocked-by: [{slug}.s2]
files: [src/x.rs, tests/x.rs]
branch: <type>/{slug}
stop: no
---
2026-01-02T14:02 spawn agent=impl
2026-01-02T14:12 checks=pass crates=<packages tested>
```

Stage field `title` = the stage's display name, from the `## Shared` stage table of `plan.md`.

Log line = `<ISO minute> <event> [key=value …]`; `ferrule board vocab` lists the events. Every line an agent writes goes through `ferrule board log <card-id> <event> [k=v …] [-- <text>]`; "log <event>" in this guide means that call. The events marked `ferrule` in `ferrule board vocab` (`gauntlet=`, `pr=`, `ci=`, `moved=`, …) are logged only by the ferrule command that owns them; `board log` refuses them with `reason=owner` and names that command. A free-text remark on any card (a rename, a correction) is `note -- <text>`, never starting with an event's name — `ferrule board log` refuses it with `reason=typed use=<event>`. A mid-run addition to a stage's `files` is logged on its card as `files+=<path>[, <path>] user=approved reason=…`, a minor handed to another stage as `minor-moved=<verdict line>` (on the stage that gave it up). Every line is stamped in UTC, so a resumed session can order events across cards.

Parent fields: `kind: epic|slice` (absent on a plain run), `epic` (slice run: the epic's issue number), `slices: [<n>, …]` (epic run: the child issues, in slice order), `issue`, `pr` (number, set when the draft opens), `branch`, `lane: fast|full` (set at the first approval stop), `gate1` approval date, `with-arch` (fast lane, until the architecture stop), `with-plan` (fast lane without an architecture step, until gate 2's stop) or `skipped`, `arch` approval date, `no-arch` or `skipped` (fast lane without an architecture step), `gate2` approval date, `split: [<n>, …]` (issues split off), `follow-ups: [<n>, …]`, `upstream: [<repo>#<n>, …]`. `ferrule board field {slug} <field>` reads one; `ferrule run init`, `ferrule gate approve` and `ferrule issue post` set most of them. Never the goal or normative text — the issue holds the goal, `{run_dir}` the spec.

| Card | `open` | `inprogress` | `inreview` | `done` |
|---|---|---|---|---|
| stage | created from plan | agent took it | green; under review (fast lane: already committed) | committed, pushed and orchestrator-verified; full lane also reviewed and approved or auto-continued |
| parent | gate 1, architecture or gate 2 pending | implementing | gate 3/4, until `ferrule run cleanup` archives it; `inprogress` again for a re-plan asked at gate 4 | — never: the run ends in `ferrule run cleanup` |

Rules:
- **No agent writes its own `done`** — the orchestrator's move (table above).
- **Runnable** = every `blocked-by` card `done`.
- Blocking derives from `blocked-by`; there is no blocked state.
- Reconciling cards with git: `ferrule guide show resume`. Cards end in `ferrule guide show cleanup`.
