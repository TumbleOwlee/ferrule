# stage-review — Per-stage review and the stage stop

<!-- lane: full -->
## Per-stage review

Full lane. Every green stage, before its approval stop:
- Fresh `fe-reviewer`. Base ref = previous stage's commit (branch point for the first stage); scope = that stage id; `ferrule guide show reviewer --section 'Axes, reported separately'` and `## Spec changes` on one stage's diff.
- `clean`, or `findings` with `count=0` → approval stop, forward the gauntlet line; minors per `ferrule guide show verify`.
- `count>0` → card → `inprogress`, resume implementer with `review.md` path and stage id, re-run `ferrule ci gauntlet {slug}`, follow-up pass by the same reviewer; rounds per `ferrule guide show verify`. An unreviewed stage is never committed.
- **The last code stage**: review in scope `branch` — it is gate 3's review; its status line → `ferrule gate next {slug} gate3 …` (it answers `stage-stop` or gate 3's stop).

<!-- lane: full -->
## Stage stop

A green, clean-reviewed stage: `ferrule stage stop {slug} s<n> --threads <n>` (`--threads` from `ferrule pr feedback route {slug}`, `ferrule guide show feedback`) decides and logs it on the card. Its rules, first hit wins: a `Needs user` item, Verification beyond unit tests, open PR threads, a plan-marked stage, a diff over 400 lines or 8 files, three auto-continues since the last stop; `s0` never stops.
- The last code stage: `auto` → commit, `ferrule pr push {slug}`, gate 4; `stop=1` → `ferrule prompt render {slug} gate3`; any other `stop=` → the `stage` stop; approval (`ferrule gate approve {slug} stage`) commits it, then gate 4.
- `auto` → the commit and push of `ferrule guide show stages --section 'Full lane'` steps 2–3, then `ferrule prompt render {slug} auto --stage s<n>`: one line naming the pushed sha, no answer awaited, the card move having updated the draft PR. A user message while a stage is running is read before the next auto-continue; `stop` holds at the next green.
- `stop=<rule> reason=<word> [detail=…]` → `ferrule prompt render {slug} stage --stage s<n> --reason <word> [--detail …]`; approved → `ferrule gate approve {slug} stage`, the commit and push of *Full lane* steps 2–3.
