# Gates

## Early stops

**GA-R-116** — A brief of at most 3 entries gets one combined early stop.

**GA-R-117** — A brief of more than 3 entries gets separate early stops.

**GA-R-118** — A brief of one entry or none skips architecture.

**GA-R-126** — A run without a brief gets the combined early stop when its architecture has at most 3 new or changed components.

**GA-R-127** — A run without a brief gets separate early stops when its architecture has more than 3 new or changed components.

**GA-R-128** — A run without a brief or an architecture step has gate 2's stop as its one early stop.

**GA-R-122** — After the combined early stop, gate 2 is a notice unless the plan stop was asked for, the plan review leaves a `Needs user` item or the run reuses an issue.

**GA-R-129** — The combined early stop offers the user a separate plan stop.

**GA-R-123** — A run without an architecture step approves its brief at gate 2's stop, together with the plan and the issue.

**GA-R-119** — `ferrule gate approve` has no `--lane` flag.

**GA-R-130** — `ferrule prompt render` has no `--recommend` flag.

**GA-R-132** — `ferrule gate approve` has no `delta` gate.

**GA-R-133** — `ferrule prompt render` has no `delta` stop.

**GA-R-121** — A run's card has no lane field.

## Stage gate

**GA-R-001** — `ferrule gate approve <slug> stage` is approved while the parent card is `inprogress` or `inreview`.

**GA-R-002** — `ferrule gate approve <slug> stage` is refused with `reason=state` while the parent card is in any other state.

**GA-R-003** — The `ferrule gate approve` help lists `inprogress` and `inreview` as the parent states of the stage gate.

**GA-R-004** — The stage-review guide directs the last code stage to commit and push the stage, or its review fixes when that stage closes a batch, before gate 4, whether or not its stage stop is shown.

**GA-R-005** — The stage stop's `Approving will:` list names committing and pushing the stage on every stage reviewed on its own, and committing and pushing the review fixes at a batch's review point.

**GA-R-006** — The stage stop's `Approving will:` list names spawning the next stage only while a later stage exists.

**GA-R-007** — The stage stop's `Approving will:` list names gate 4 as the next stop on the last code stage.

## Branch review

**GA-R-008** — The reviewer guide's branch scope directs the review to read the worktree's uncommitted changes as well as the branch's commits.

**GA-R-009** — The reviewer guide's branch scope directs the review to read whole the untracked files the uncommitted diff only lists.

**GA-R-010** — The reviewer guide's `## Spec changes` directs the branch review to read the uncommitted spec changes as well as the committed ones.

**GA-R-078** — Gate 3 always reviews the whole branch, whatever the stages' tiers.

**GA-R-079** — Each pass of the gate 3 review uses a fresh reviewer.

**GA-R-094** — The reviewer's gate 3 branch review status line reports its minor count as `minors=<n>`.

**GA-R-096** — When a clean gate 3 branch review reports `minors=` above 0, `ferrule gate next` answers a step that resumes the implementer with the minors before the review fixes are committed.

**GA-R-125** — A `Needs user` item the branch review leaves is asked on gate 4's stop, never a stop of its own.

## Stage review

**GA-R-011** — The reviewer guide's stage scope directs the review to read the worktree's uncommitted changes as well as the commits since its base.

**GA-R-012** — The reviewer guide's stage scope directs the review to read whole the untracked files the uncommitted diff only lists.

**GA-R-013** — The reviewer guide's `## Spec changes` directs the stage review to read the uncommitted spec changes as well as the committed ones.

**GA-R-076** — The fixes from a batch review land as one commit at the review point.

**GA-R-077** — Minor findings are fixed by the implementer of that stage before the commit.

**GA-R-080** — A batch's review point covers the commits of every stage in the batch.

**GA-R-095** — When a clean stage or batch review reports `minors=` above 0, `ferrule gate next` answers a step that resumes the stage's implementer with the minors before the stage stop (GA-R-077).

**GA-R-097** — After the minors fix step, the next step is the stop, with no second review.

**GA-R-098** — The stop that follows a minors fix step states how many minors were fixed.

**GA-R-099** — `ferrule gate next <slug> stage` takes a stage or batch review's status line; `findings` with `count=` above 0 answers `next=fix`.

**GA-R-102** — The auto-continue notice that follows a minors fix step states how many minors were fixed.

**GA-R-103** — `ferrule gate next <slug> stage` logs `review=clean scope=stage` on the run card for a clean stage review and `review=clean scope=batch` for a clean batch review.

## Review rules

**GA-R-014** — The reviewer guide reads from the specs README only `Rules for writing specs`.

**GA-R-015** — The implementer guide's `## TDD` section states the fixed test-first order.

**GA-R-017** — The reviewer guide's Standards axis checks every rule `AGENTS.md` states, under any heading.

**GA-R-018** — The reviewer guide takes a comment violation's severity from the project's comment rule.

**GA-R-019** — The reviewer guide rates a comment violation a minor when the project states no comment rule.

**GA-R-064** — The reviewer guide does not ask the review to judge test-first order.

**GA-R-065** — The reviewer guide gives the review the card's files-outside, pins-existing and replaced-test notes; a finding may concern the change they point at, and the notes never block or reach a user stop.

**GA-R-067** — The reviewer guide rates a plan bookkeeping gap as no finding, so it blocks nothing and never reaches the verdict.

**GA-R-092** — The reviewer's stage and batch status line reports its blocker count as `blockers=<n>` and its minor count as `minors=<n>`.

**GA-R-100** — At the `stage`, `gate3`, `gate2` and `replan` positions, a `clean` status line, or `findings` with `count=0`, that lacks a numeric `minors=` is a usage error (exit 2).

**GA-R-101** — `ferrule gate next` logs a clean review's minor count above 0 as `minors=<n>` on its `review=` card event.

**GA-R-138** — The reviewer's plan review status line reports its minor count as `minors=<n>`.

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

**GA-R-120** — `ferrule guide show` has no `--lane` flag.

**GA-R-134** — No guide topic names a `delta` review scope, stop or gate.

## Plan gate

**GA-R-062** — The repository for a live check is named at gate 2.

**GA-R-066** — The implementer guide has a plan bookkeeping gap — a stage's ID→test table or the plan's stage table missing or misnaming a test, ID or file the stage has — logged as a `note` on the stage card, never as a reason to stop.

**GA-R-140** — When a clean plan review, or one with `count=0` and a `Needs user` item, reports `minors=` above 0, `ferrule gate next <slug> gate2` answers a step that resumes the planner with the minors before gate 2's stop or notice.

**GA-R-141** — After the gate 2 minors fix step, the next step is gate 2's stop or notice, with no second plan review.

**GA-R-142** — Gate 2's stop or notice that follows a minors fix step states how many minors were fixed.

**GA-R-135** — When a clean re-plan review, or one with `count=0` and a `Needs user` item, reports `minors=` above 0, `ferrule gate next <slug> replan` answers a step that resumes the planner with the minors before the re-plan stop or notice.

**GA-R-136** — After the re-plan minors fix step, the next step is the re-plan stop or notice, with no second plan review.

**GA-R-137** — The re-plan stop or notice that follows a minors fix step states how many minors were fixed.

**GA-R-139** — A gate 2 minors fix step leaves gate 2 a notice when it would have been one without minors (GA-R-122).

## Stage tiers

**GA-R-069** — The planner proposes one tier per stage: own review, batched or skipped.

**GA-R-070** — The gate 2 stop lets the user change each stage's tier.

**GA-R-071** — A stage with host writes, persisted state or new public surface cannot take the skipped tier.

**GA-R-072** — A stage that changes the spec gets its own review, whatever its planned tier.

**GA-R-073** — A blocker found in a batch review gives the next stage its own review.

**GA-R-074** — A batch closes at 3 stages or 1,200 changed lines, whichever comes first.

**GA-R-075** — A batched stage commits and pushes when its checks pass, with no stop unless its implementer has open questions, which reach the user in one batch before the commit.

**GA-R-081** — A slice run takes its stages' tiers from its own plan.

**GA-R-086** — A batch is a run of consecutive batched stages; it closes before a stage of another tier.

**GA-R-087** — A batch open at the run's last code stage closes there.

**GA-R-088** — A skipped stage commits and pushes when its checks pass, with no stop and no review before gate 3, unless its implementer has open questions, which reach the user in one batch before the commit.

**GA-R-089** — A stage marked to stop at gate 2 takes the own tier.

**GA-R-093** — A board rebuilt from the draft PR gives each stage the tier and stop mark settled at gate 2.

**GA-R-124** — A stage's tier alone decides its review, whatever the brief's size.

**GA-R-131** — The stages guide directs a batched or skipped stage whose question answers come after its tier decision to run `ferrule stage tier` again before its commit.

## Checks

**GA-R-082** — Every stage runs the format check and its new tests.

**GA-R-083** — A review point runs the tests of the changed crates and clippy.

**GA-R-084** — Gate 3 runs the full gauntlet.

**GA-R-085** — The implementer does not run a suite the workflow's own checks already ran.

**GA-R-090** — `ferrule ci gauntlet` at the full level logs its pass or failure on the run card when the worktree holds no uncommitted change.

**GA-R-091** — A gauntlet step's `{crates}` argument runs at a review point as the crates the diff since the review point's base touches.

**GA-R-115** — A batched or skipped stage's checks also run the review-level steps, `{crates}` as the crates its own diff touches.

## Stage questions

**GA-R-104** — The implementer asks its open questions in one batch before review.

**GA-R-105** — The implementer asks a question at once only when it is blocked.

**GA-R-106** — Items a reviewer raises for the user are relayed after the first review pass.

**GA-R-107** — A user's answer that changes code gets a recheck of that change only.

**GA-R-108** — A user's answer that changes no code triggers no review rerun.

## Stage stops

**GA-R-109** — A spec change with `origin=user` counts as kept.

**GA-R-110** — An item the user has settled does not reappear at a later stop.

**GA-R-111** — A `Confirm:` line that has been superseded does not reappear at a later stop.

**GA-R-112** — A stage stop is shown only when the user has a real decision to make; otherwise the run continues.

**GA-R-113** — No stop is triggered by the number of lines a stage changed.

**GA-R-114** — A missing `glow` does not stop the run; stops are shown as plain text instead.

## Design checks

**GA-R-143** — After the architect answers `ready`, a comprehension check runs before the architecture stop is shown.

**GA-R-144** — The comprehension check reads only the run's `spec-diff.md` and `architecture.md`, with no project instruction file in its context.

**GA-R-145** — The comprehension check states in its own words what the change proposes and how the existing system works.

**GA-R-146** — The comprehension check reports a finding for each section it cannot paraphrase and each term the documents use without defining.

**GA-R-147** — After the architect answers `ready`, a zoom-out check runs before the architecture stop is shown.

**GA-R-148** — The zoom-out check reports a finding where the change is not needed, could be simpler, or is already covered by existing code or a dependency.

**GA-R-149** — Each zoom-out finding names what could be removed or reused.

**GA-R-150** — The zoom-out check may read the project's code.

**GA-R-151** — Neither design check changes any file but its own result file.

**GA-R-152** — The architecture stop shows every finding of both checks in full, numbered per check as C<n> for comprehension and Z<n> for zoom-out.

**GA-R-153** — The architecture stop names the file paths of the comprehension paraphrase and the zoom-out reasoning.

**GA-R-154** — The checks add no stop: the architecture stop is shown after them whatever their findings.

**GA-R-155** — A run whose architecture is skipped at gate 1 runs neither check (GA-R-118).

**GA-R-160** — A run whose brief is approved at gate 2's stop runs neither check (GA-R-123).

**GA-R-156** — Each time the architecture stop is reached, both checks run on the current `spec-diff.md` and `architecture.md`.

**GA-R-157** — A check that produces no result leaves the architecture stop shown as usual, with a line saying that check did not run.

**GA-R-158** — The combined early stop counts as the architecture stop for every design-check entry (GA-R-116, GA-R-126).

**GA-R-161** — An epic run's epic stop counts as the architecture stop for every design-check entry.

**GA-R-159** — A check that produced a result without findings is shown at the architecture stop with a line saying it found none.
