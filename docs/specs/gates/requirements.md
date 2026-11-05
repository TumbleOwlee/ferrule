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

**GA-R-016** — The reviewer guide's TDD honesty axis states the TDD rules itself, without sending the review to another guide.

**GA-R-017** — The reviewer guide's Standards axis checks every rule `AGENTS.md` states, under any heading.

**GA-R-018** — The reviewer guide takes a comment violation's severity from the project's comment rule.

**GA-R-019** — The reviewer guide rates a comment violation a minor when the project states no comment rule.

## Guide rules

**GA-R-020** — The author, architect, planner, implementer and reviewer guides each take the rules `AGENTS.md` states as binding, naming no category of rule.

**GA-R-021** — No shipped guide cites an `AGENTS.md` heading for a rule.

**GA-R-022** — No shipped guide cites a numbered rule of the specs README.

**GA-R-023** — The principles guide defines a non-behavior change as a change that needs no observable behavior to change.

**GA-R-024** — The author, architect and planner guides each state the local ID forms and their numbering.

**GA-R-025** — The implementer guide states that a stage changes only the files its `files` list names.

**GA-R-026** — The reviewer guide rates a file in the diff outside the in-scope stages' `files` lists a blocker.

**GA-R-027** — The author guide states how to read an issue's body and comments.

**GA-R-028** — The planner guide states that an entry too coarse is split so each entry holds one rule.
