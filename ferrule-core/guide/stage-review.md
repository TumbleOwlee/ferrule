# stage-review — Per-stage review and the stage stop

<!-- lane: full -->
## Per-stage review

Full lane. A review point is the stage `ferrule stage tier {slug} s<n>` names (`review=stage`) or the batch it closes (`review=batch stages=…`); a stage with `review=none` has none.
- Fresh `fe-reviewer`. Scope `stage s<n>`: base = the previous stage's commit (branch point for the first stage). Scope `batch s<a>,s<b>`: base = the commit before `s<a>`, the review reads the commits of every listed stage and the uncommitted fixes. Both scopes run `ferrule ci gauntlet {slug} --level review` first.
- `clean`, or `findings` with `count=0` → the `Stage stop` rules below, for a stage or a batch alike (a batch's stop prompt names its stages); forward the gauntlet line; minors per `ferrule guide show verify`.
- `count>0` → card → `inprogress`, resume the implementer with the `review.md` path and stage id, who fixes every finding, minors included; card → `inreview`, re-run `ferrule ci gauntlet {slug} --level review`, follow-up pass by the same reviewer; rounds per `ferrule guide show verify`. A batch's fixes are committed once, when clean, on the last stage's card, and pushed.
- A batch review status line with `blockers=` above 0 → `ferrule stage tier {slug} s<next> --after-blocker` before the next stage starts: that stage gets its own review.
- **The last code stage's review point**: scope `branch`, each pass a fresh reviewer — it is gate 3's review; its status line → `ferrule gate next {slug} gate3 …` (it answers `stage-stop` or gate 3's stop).

<!-- lane: full -->
## Stage stop

The rules run at review points only. A green, clean-reviewed stage at one: `ferrule stage stop {slug} s<n> --threads <n>` (`--threads` from `ferrule pr feedback route {slug}`, `ferrule guide show feedback`) decides and logs it on the card. Its rules, first hit wins: a `Needs user` item, Verification beyond unit tests, open PR threads, a plan-marked stage, a diff over 400 lines or 8 files, three auto-continues since the last stop; `s0` never stops.
- The last code stage: `auto` → commit and push the stage, or its review fixes when it closes a batch, before gate 4: commit, `ferrule pr push {slug}`, gate 4; `stop=1` → `ferrule prompt render {slug} gate3`; any other `stop=` → the `stage` stop; approved → `ferrule gate approve {slug} stage`, commit, `ferrule pr push {slug}`, gate 4.
- `auto` → the commit and push of `ferrule guide show stages --section 'Full lane'` — step 2 at a stage review, step 4 at a batch review — then `ferrule prompt render {slug} auto --stage s<n>`: one line naming the pushed sha, no answer awaited, the card move having updated the draft PR. A user message while a stage is running is read before the next auto-continue; `stop` holds at the next green.
- `stop=<rule> reason=<word> [detail=…]` → `ferrule prompt render {slug} stage --stage s<n> --reason <word> [--detail …]`; approved → `ferrule gate approve {slug} stage`, the commit and push of *Full lane* (step 2 at a stage review, step 4 at a batch review).
