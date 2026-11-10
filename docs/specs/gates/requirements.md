# Gates

## Stage gate

**GA-R-001** — `ferrule gate approve <slug> stage` is approved while the parent card is `inprogress` or `inreview`.

**GA-R-002** — `ferrule gate approve <slug> stage` is refused with `reason=state` while the parent card is in any other state.

**GA-R-003** — The `ferrule gate approve` help lists `inprogress` and `inreview` as the parent states of the stage gate.

**GA-R-004** — The stage-review guide directs the last code stage's approved stage stop to commit and push the stage before gate 4.

**GA-R-005** — The stage stop's `Approving will:` list names committing and pushing the stage on every stage.

**GA-R-006** — The stage stop's `Approving will:` list names spawning the next stage only while a later stage exists.

**GA-R-007** — The stage stop's `Approving will:` list names gate 4 as the next stop on the last code stage.

## Branch review

**GA-R-008** — The reviewer guide's branch scope directs the review to read the worktree's uncommitted changes as well as the branch's commits.

**GA-R-009** — The reviewer guide's branch scope directs the review to read whole the untracked files the uncommitted diff only lists.

**GA-R-010** — The reviewer guide's `## Spec changes` directs the branch review to read the uncommitted spec changes as well as the committed ones.

## Stage review

**GA-R-011** — The reviewer guide's stage scope directs the review to read the worktree's uncommitted changes as well as the commits since its base.

**GA-R-012** — The reviewer guide's stage scope directs the review to read whole the untracked files the uncommitted diff only lists.

**GA-R-013** — The reviewer guide's `## Spec changes` directs the stage review to read the uncommitted spec changes as well as the committed ones.

## Review rules

**GA-R-014** — The reviewer guide reads from the specs README only `Rules for writing specs`.

**GA-R-015** — The implementer guide's `## TDD` section states the fixed test-first order.

**GA-R-017** — The reviewer guide's Standards axis checks every rule `AGENTS.md` states, under any heading.

**GA-R-018** — The reviewer guide takes a comment violation's severity from the project's comment rule.

**GA-R-019** — The reviewer guide rates a comment violation a minor when the project states no comment rule.

**GA-R-064** — The reviewer guide does not ask the review to judge test-first order.

**GA-R-065** — The reviewer guide gives the review the card's files-outside, pins-existing and replaced-test notes; a finding may concern the change they point at, and the notes never block or reach a user stop.

**GA-R-067** — The reviewer guide rates a plan bookkeeping gap as no finding, so it blocks nothing and never reaches the verdict.

## Guide rules

**GA-R-020** — The author, architect, planner, implementer and reviewer guides each take the rules `AGENTS.md` states as binding, naming no category of rule.

**GA-R-021** — No shipped guide cites an `AGENTS.md` heading for a rule.

**GA-R-022** — No shipped guide cites a numbered rule of the specs README.

**GA-R-023** — The principles guide defines a non-behavior change as a change that needs no observable behavior to change.

**GA-R-024** — The author, architect and planner guides each state the local ID forms and their numbering.

**GA-R-025** — The implementer guide states that a stage's `files` list is advisory and a file outside it is noted on the card.

**GA-R-027** — The author guide states how to read an issue's body and comments.

**GA-R-028** — The planner guide states that an entry too coarse is split so each entry holds one rule.

**GA-R-029** — An agent's guide cites only the guide of another of that agent's jobs.

**GA-R-030** — A workflow topic cites no agent's guide.

**GA-R-031** — The author, architect, planner and implementer guides each state the `question.md` shape, batching and `Why you:` rule.

**GA-R-032** — The `reviewer-plan` guide states the stage title rules.

**GA-R-033** — The reviewer guide's follow-up pass names the plan checks by their guide topic.

**GA-R-034** — The implementer guide states that a test's doc comment lists the IDs its assertions directly verify, and only those.

**GA-R-035** — The reviewer guide's Standards axis states the same ID citation rule as the implementer guide.

**GA-R-036** — The implementer guide reads `Rules for writing specs` before a spec change.

**GA-R-037** — The bootstrap skill suggests no `## Project rules` override.

**GA-R-038** — The `planner-replan` guide states the `question.md` shape.

**GA-R-039** — A command's help cites no agent's guide.

**GA-R-040** — The `ferrule stage commit` help cites the `stages` topic.

**GA-R-041** — The help of `ferrule run diff`, `run log`, `run plan-diff`, `spec related`, `spec next` and `spec refs` has no `See:` line.

**GA-R-042** — A shipped skill other than `fe-review` cites no agent's guide.

**GA-R-043** — The README cites no agent's guide.

**GA-R-044** — The author, architect, planner and implementer guides each state the `decisions.md` line shape and its rules.

**GA-R-045** — The `author-late`, `architect-epic`, `planner-replan` and `reviewer-plan` guides each cite no other guide.

**GA-R-046** — The `author-late` guide states the author's working rules, `AGENTS.md` handling and the split brief.

**GA-R-047** — An agent file names one guide for each of that agent's jobs.

**GA-R-048** — `ferrule guide show decisions` refuses the topic as unknown.

**GA-R-049** — `ferrule guide list` lists no `decisions` topic.

**GA-R-050** — The shipped guide sources hold the `decisions.md` line shape and its rules once.

**GA-R-051** — The shipped guide sources hold once any text a single-job guide prints that its agent's main guide also prints.

**GA-R-052** — The `architect-epic` guide states the architect's procedure, the `decisions.md` line shape and the `architecture.md` shape.

**GA-R-053** — The `planner-replan` guide states the planner's procedure, the `decisions.md` line shape and the `plan.md` shape and rules.

**GA-R-054** — The `reviewer-plan` guide states the reviewer's project rules, inputs, follow-up pass and output shape.

**GA-R-055** — The shipped guide sources hold the `question.md` batch and its rules once.

**GA-R-056** — The shipped guide sources hold the ID citation rule once.

**GA-R-057** — The shipped guide sources hold the line `**Concise, compact, facts only.**` once.

**GA-R-058** — Ferrule's sources hold once a paragraph an installed agent file shares with a guide.

**GA-R-059** — No heading section of ferrule's shipped guide or installed-file sources repeats the text of another.

**GA-R-060** — The planner guide reads `Rules for writing specs` before a spec edit.

**GA-R-061** — The `planner-replan` guide reads `Rules for writing specs` before a spec edit.

**GA-R-063** — The implementer guide directs the implementer to check, before review, that each cited spec ID is asserted by its test.

**GA-R-068** — An interface sketch in the architecture binds only what the architecture's `## Public surface` lists.

## Plan gate

**GA-R-062** — The repository for a live check is named at gate 2.

**GA-R-066** — The implementer guide has a plan bookkeeping gap — a stage's ID→test table or the plan's stage table missing or misnaming a test, ID or file the stage has — logged as a `note` on the stage card, never as a reason to stop.
