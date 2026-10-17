# approval — Rendering a stop and recording the answer

## Approval prompt

Every stop is one message rendered by `ferrule prompt render <kind> {slug} [k=v …]` and relayed verbatim; the orchestrator composes none of it. It picks the files the stop shows from the cards (`spec-diff.md` while the brief is unapproved or its rev moved since the last approval), opens them in the viewer, quotes their lines in fixed order, adds what approving will do, and logs the stop. Kinds and their arguments: `ferrule prompt render --help`; each stop's topic names its call.

Two shapes, one answer grammar each:
- **Approval** — gate, architecture, epic, stage and re-plan stops. Ends in `Approving will:`, `Options:` (numbered, when the stop has any) and `Answer: approve [<n>[: <value>] …] · change <what> · stop`. `approve 2` takes option 2; `change` goes to the agent that wrote the file; `stop` halts and keeps the board (*Resume an interrupted run* (`ferrule guide resume`)). A `Needs user` item is answered in the same reply by its option letter (`approve · a`; several items: one letter each, in order) and relayed with the approval. `re-plan: <why>` is accepted at any stage stop, at gate 4's stop and in reply to an auto-continue line (*Re-plan* (`ferrule guide replan`)).
- **Question** — an agent's decision batch, a blocked stage, a stage's meaningful spec changes. Numbered decisions, lettered options, `Answer: <1: a · 2: b · …> · other <text>`.

Notices (`auto`, `gate2-notice`, `replan-notice`) are one line, no answer awaited, never logged as a stop.

Approved → `ferrule run approve <gate> {slug} [k=v …]` first (gate ∈ `gate1`, `arch`, `epic`, `gate2`, `stage`, `replan`, `delta`, `gate3`, `gate4`): it records the gate's date on the parent card, does what that gate's topic says approval does, and logs `spec-rev=`/`arch-rev=` for the files the stop showed and `decisions-seen=` for `decisions.md`, so the next stop shows only the files that moved and confirms only the decisions written since.
