# stages — Implement, stage by stage

## Every stage

- A fresh `fe-implementer`, spawned with the worktree path `{worktree}`, the plan path `{run_dir}/plan.md`, its stage card ids and `lane=fast|full`.
- **At most 3 code stages per implementer** (`s0` does not count). The next stage gets a fresh one, which starts from its cards and plan sections, never from its predecessor's context. Fixes for an earlier stage (gate 3 findings, PR feedback, a red gauntlet) go to the current implementer, briefed with that stage's card.
- The implementer records each new test's proof: `ferrule stage red <card> <test>` logs a failure the run reports (`--mutation <text>` records a mutation proof instead), `ferrule stage pins <card> <test> -- <text>` marks a test that pins existing behaviour. On a Rust project `ferrule stage commit` refuses a new test without one. Both commands' help cites this topic.
- Not done until the plan's Verification method has run and its outcome is reported. Waiving it requires asking.
- **The draft PR body is the board, rendered.** Once the PR exists, every `ferrule board move` re-renders it and pushes it when it changed (`pr-body=pushed|unchanged`). It is the living PR: status line, `## Why`, `## Scenarios`, `## Stages` (each stage's verification as sub-items under its line, stage ids only in HTML comments), `## Review here`, then the sections the gate 4 body shares: `## Requirements`, `## Spec changes since filing`, `## Architecture` (the current `architecture.md` rev) and `## Decisions` (`decisions.md`'s design lines and its chosen spec lines not already under spec changes). `ferrule pr body {slug} draft` prints it.
- `## Spec changes since filing` in the draft = the cards' `spec-delta` lines whose `**<ID>**` the committed `git diff {main}...<branch> -- docs/specs` touches, last line per ID; omitted without a `branch` or when git fails. IDs added after filing show only under spec changes until gate 4.
- `pr-body=refused reason=stage-ref stage-ref=<token>`: the token sits in a card's `spec-delta` why or a `decisions.md` line; resume the agent that wrote it to re-log the `spec-delta` (last line per ID wins) or reword its line; never edit the body by hand; the PR keeps its previous body until then.
- **Stage checks, both lanes:** once a stage is green the orchestrator runs `ferrule ci gauntlet {slug} --level push --stage s<n>` (the push-level steps and the tests the stage's diff adds; a batched, skipped or fast-lane stage also the review-level steps over the crates its diff touches). Red → card → `inprogress`, resume the implementer with the line's `step=` and `log=`.
- **Every push is `ferrule pr push {slug}`** (push is never the implementer's): it refuses (`push=refused reason=…`, the line's `hint=` names the fix; `ferrule pr push --help` lists every reason) unless the worktree's head is a stage's verified commit (`ferrule stage verify`: the commit exists, is on the run's branch and is the card's last `commit=`); it runs only the push-level steps, pushes, logs `pushed=<sha>` on the parent card, prints `checks=fast` and updates the draft PR — **the first push opens it** (`pr-body=opened pr=<n>`, `pr` recorded, `pr=draft` logged). A pushed commit is verified. The user reviews there (`ferrule guide show feedback`).
- `push=refused reason=sha mismatch=<why>` → *Question and blocked stage* with the refusal's `mismatch=`: continue with the next stage or fix it outside ferrule, never a re-plan. `reason=gauntlet` → card → `inprogress`, resume the implementer with the line's `step=` and `log=`.
- `push=refused reason=diverged local=<sha> remote=<sha>` → *Question and blocked stage* with the two heads: rebase through ferrule or resolve it outside ferrule, never force-push by hand, never a re-plan.

<!-- lane: fast -->
## Fast lane

Tiers do not apply: no tier, no per-stage review point. Per stage:

1. Green: card → `inreview`, the stage checks (*Every stage*), the implementer commits with `ferrule stage commit` (it refuses from `inprogress` and alone logs `commit=`), answers `status=committed stage=s<n> sha=<sha> delta=<n>` — no per-stage code review, no approval stop.
2. `ferrule pr push {slug}`. `delta>0` → fresh `fe-reviewer`, scope `delta s<n>` (`ferrule guide show spec-changes`): `count>0` → resume the implementer with the `review.md` path, fixes committed on green, push, follow-up pass by the same reviewer, rounds per `ferrule guide show verify`; a `Needs user` item → the delta stop below. Then card → `done`, the next stage (*Every stage*).
3. Last stage `done` → `ferrule guide show feedback`, then `ferrule guide show gate3`: its branch review is the only code review of a fast-lane run.

**Stop (delta):** `ferrule prompt render {slug} delta --stage s<n>` — the verdict's `Needs user` items as a question batch; the answers (keep · revert · split) go to the implementer, then the next stage.

Fixes in the fast lane (gate 3 findings, PR feedback, red gauntlet) are committed on green too, card → `inreview` first: the implementer answers `status=committed`, the orchestrator pushes before any re-review — a review diffs commits, never a dirty worktree.

<!-- lane: full -->
## Full lane

Per stage:

1. Green: card → `inreview`, `status=inreview stage=s<n>`. The orchestrator runs the stage checks (*Every stage*), then `ferrule stage tier {slug} s<n>`, which prints `tier=<t> review=<point>`.
2. `review=stage` → the per-stage review (`ferrule guide show stage-review`); both clean → its `Stage stop` rules decide: a rule fires → user approval stop; none fires → auto-continue (no stop). Either way the same implementer (resumed) commits, answers `status=committed`; then `ferrule pr push {slug}`, card → `done`.
3. `review=none` → no review, no stop: open implementer questions → one question stop before the commit (`ferrule guide show question`); then resume the implementer to commit, `ferrule pr push {slug}`, card → `done`, the next stage.
4. `review=batch stages=s<a>,s<b>` → open implementer questions → one question stop before the commit (`ferrule guide show question`); then resume the implementer to commit, `ferrule pr push {slug}`, the card stays `inreview`; then the batch review (`ferrule guide show stage-review`), clean → its `Stage stop` rules decide as in step 2 (approval stop or auto-continue), the review fixes of the batch committed and pushed, card → `done`. The batch's earlier stages are already `done` (step 3).

## Question and blocked stage

Either lane.
- `status=question` → `ferrule guide show question`, headed with the stage id; the answers go to the same implementer.
- `status=blocked` → one stop, the card stays where it is: `ferrule prompt render {slug} blocked --stage s<n> --reason <reason>`. `1: a <text>` → relayed to the same implementer; `1: b` → `ferrule guide show replan`, the reason verbatim; `1: c` → the run halts, the board stays for `ferrule guide show resume`.
- A blocked reason that suggests `--adopt <sha>` takes this same stop with its standard options; `1: a` relays to the same implementer, which runs `ferrule stage commit <card> --adopt <sha>` and reports `status=committed`. The adopted commit then takes the stage checks, `ferrule stage tier` and its review point like any commit.
- A diverged push refusal → one stop, the card stays where it is: `ferrule prompt render {slug} blocked --local <sha> --remote <sha>`, the heads from the refusal line; the stop says to rebase through ferrule or resolve it outside ferrule and never force-push by hand. `1: a` → `ferrule run rebase {slug}`, then `ferrule pr push {slug}`; `1: b` → `ferrule pr push {slug}` again, the same refusal returning to the same stop; `1: c` → the run halts, the board stays for `ferrule guide show resume`.
- A sha push refusal → one stop, the card stays where it is: `ferrule prompt render {slug} blocked --mismatch <why>`, `<why>` from the refusal line's `mismatch=`; the stop says the push stays refused until a stage commit lands on top of the current head. `1: a` → the refused stage's flow goes on past its push as its lane says, then the first stage in plan order whose card has no `commit=` (`ferrule board show {slug}`): its implementer resumed when the card is `inprogress`, else started per *Every stage*, and `ferrule pr push {slug}` after its commit; `1: b` → `ferrule pr push {slug}` again, the same refusal returning to the same stop; `1: c` → the run halts, the board stays for `ferrule guide show resume`.
- `rebase=conflict paths=<n>` → resume the implementer briefed with the card of the stage whose `commit=` is the newest on the run's cards (`ferrule board events {slug}`), the line and the worktree; it resolves the conflict and answers `status=rebased stage=s<n>`.
- `status=rebased` → `ferrule run rebase {slug}` again, then `ferrule pr push {slug}`; a `rebase=refused` line → its `hint=` names the fix.
