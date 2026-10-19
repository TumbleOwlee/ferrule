# stages — Implement, stage by stage

## Every stage

- A fresh `fe-implementer`, spawned with the worktree path `{worktree}`, the plan path `{run_dir}/plan.md`, its stage card ids and `lane=fast|full`. Its first step is `ferrule guide implementer --stage s<n>`.
- **At most 3 code stages per implementer** (`s0` does not count). The next stage gets a fresh one, which starts from its cards and plan sections, never from its predecessor's context. Fixes for an earlier stage (gate 3 findings, PR feedback, a red gauntlet) go to the current implementer, briefed with that stage's card.
- Not done until the plan's Verification method has run and its outcome is reported. Waiving it requires asking.
- **The draft PR body is the board, rendered.** Once the PR exists, every `ferrule card move` re-renders it and pushes it when it changed (`pr-body=pushed|unchanged`). It is the living PR: status line, `## Why`, `## Scenarios`, `## Stages` (each stage's verification as sub-items under its line, stage ids only in HTML comments), `## Review here`, then the sections the gate 4 body shares: `## Requirements`, `## Spec changes since filing`, `## Architecture` (the current `architecture.md` rev) and `## Decisions` (`decisions.md`'s design lines and its chosen spec lines not already under spec changes). `ferrule pr body {slug} draft` prints it.
- `## Spec changes since filing` in the draft = the cards' `spec-delta` lines whose `**<ID>**` the committed `git diff {main}...<branch> -- docs/specs` touches, last line per ID; omitted without a `branch` or when git fails. IDs added after filing show only under spec changes until gate 4.
- `pr-body=refused stage-ref=<token>`: the token sits in a card's `spec-delta` why or a `decisions.md` line; resume the agent that wrote it to re-log the `spec-delta` (last line per ID wins) or reword its line; never edit the body by hand; the PR keeps its previous body until then.
- **Every push is `ferrule pr push {slug}`** (push is never the implementer's): it refuses (`push=refused reason=…`, the line's `hint=` names the fix; `ferrule pr push --help` lists every reason) unless the worktree's head is a stage's verified commit (`ferrule run sha-check`: the commit exists, is on the run's branch and is the card's last `commit=`) and the gauntlet passes on it (`cached` when the commit is the reviewed tree); then it pushes, logs `gauntlet=pass cov=… sha=…` on the parent card and updates the draft PR — **the first push opens it** (`pr-body=opened pr=<n>`, `pr` recorded, `pr=draft` logged). The user reviews there (`ferrule guide feedback`).
- `push=refused reason=sha …` → *Question and blocked stage* with `reason=sha mismatch <reason>`. `reason=gauntlet` → card → `inprogress`, resume the implementer with the gauntlet line.

<!-- lane: fast -->
## Fast lane

Per stage:

1. Green: card → `inreview`, the implementer commits (`ferrule card log` refuses `commit=` from `inprogress`), answers `status=committed stage=s<n> sha=<sha> delta=<n>` — no per-stage code review, no approval stop.
2. `ferrule pr push {slug}`. `delta>0` → fresh `fe-reviewer`, scope `delta s<n>` (`ferrule guide spec-changes`): `count>0` → resume the implementer with the `review.md` path, fixes committed on green, push, follow-up pass by the same reviewer, rounds per `ferrule guide verify`; a `Needs user` item → the delta stop below. Then card → `done`, the next stage (*Every stage*).
3. Last stage `done` → `ferrule guide feedback`, then `ferrule guide gate3`: its branch review is the only code review of a fast-lane run.

**Stop (delta):** `ferrule prompt render {slug} delta --stage s<n>` — the verdict's `Needs user` items as a question batch; the answers (keep · revert · split) go to the implementer, then the next stage.

Fixes in the fast lane (gate 3 findings, PR feedback, red gauntlet) are committed on green too, card → `inreview` first: the implementer answers `status=committed`, the orchestrator pushes before any re-review — a review diffs commits, never a dirty worktree.

<!-- lane: full -->
## Full lane

Per stage:

1. Green: card → `inreview`, `status=inreview stage=s<n>`. Orchestrator runs `ferrule run gauntlet {slug}` and the per-stage review (`ferrule guide stage-review`).
2. Both clean → `ferrule guide stage-review --section 'Stage stop'` rules decide: a rule fires → user approval stop; none fires → auto-continue (no stop). Either way the same implementer (resumed) commits, answers `status=committed`.
3. `ferrule pr push {slug}`, then card → `done`.

## Question and blocked stage

Either lane.
- `status=question` → `ferrule guide question`, headed with the stage id; the answers go to the same implementer.
- `status=blocked` → one stop, the card stays where it is: `ferrule prompt render {slug} blocked --stage s<n> --reason <reason>`. `1: a <text>` → relayed to the same implementer; `1: b` → `ferrule guide replan`, the reason verbatim; `1: c` → the run halts, the board stays for `ferrule guide resume`.
