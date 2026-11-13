# epic — Epic runs and slice runs

## Epic

A feature too big for one run: the author answered `size=epic`, or the architect's size guard tripped. The epic run settles the shared core and cuts the feature into slices; it writes no code and has no plan, worktree or PR. Each slice is later a normal run on its own issue.

1. **Brief** — gate 1 as usual; its stop approves `epic`. Or the architect's size guard turned a behavior run into one: `ferrule board set {slug} kind=epic`, go to step 3; the epic stop approves the brief too when `gate1` is not yet a date.
2. **Skeleton and slices** — spawn `fe-architect` with the brief and `mode=epic`. It writes `architecture.md` and its summary for the **skeleton** only (the core abstractions every slice builds on, with their `## Interfaces`) and `slices.md`. Questions relayed as usual; it answers `status=ready … slices={run_dir}/slices.md`.
3. **Epic issue draft** — `fe-author` drafts `issue.md` (`ferrule guide show issue`: existing issues searched, `reuse` or `new`).
4. **Epic stop** — `ferrule prompt render {slug} epic [--issue <n>]` (`<n>`: the author answered `reuse`); its `Confirm:` names every brief ID no slice or two slices claim. Approved → `ferrule gate approve {slug} epic` (records `arch`, and `gate1` while pending).
5. **On approval, in order** — no further stop. On GitHub a GitHub epic is a milestone and slices are the issues in it:
   1. File the milestone: `ferrule issue post {slug} [--reuse <n>]` prints `milestone=<m>` and records `milestone`. The milestone description holds the brief and skeleton and ends in `## Changelog`; no epic issue is filed. With `--reuse <n>` the open issue is closed with a comment linking the milestone; its title and body stay.
   2. Per slice, in `slices.md` order: `ferrule issue post {slug} --slice <n>`. The issue is filed in the milestone with its `area:` labels and no marker line. An open issue in the milestone already titled exactly as the slice's is adopted, never filed twice; the number is recorded at the slice's position in the parent card's `slices:` (`[71, -, 73]`, `-` not filed yet) — a later slice's `Depends on` makes it `blocked by` the issues it names.
   3. Without milestones (a tracker that has none): `ferrule issue post {slug}` once more rewrites the epic issue, the one exception to `ferrule guide show issue`'s never-rewritten rule, so its `## Slices` list links every child. On GitHub this step does not apply.
   4. `ferrule run cleanup {slug}` — no worktree, no CI; it records the metrics; cards and artifacts are archived.
   5. One notice line: `Epic · {slug} · milestone #<m> · slices #a, #b, … filed · start with /fe-feature on #a`. Without milestones: `Epic · {slug} · #<e> · slices #a, #b, … filed · start with /fe-feature on #a`.

The human closes the milestone once its last slice merges; the epic stays open until then.

### Slice run

`/fe-feature` on a child issue ("take issue #n"), a normal run otherwise.
- **Gate 1 board:** `ferrule run init {slug} --issue <n>` already recorded `kind: slice`, `milestone` and `slice` on the card, and wrote the milestone's description to `epic.md`; there is no `slice-line.md`. Name `epic.md`'s path in every spawn brief of the run except the implementer's. Without milestones the card holds `epic: <e>` instead, `slice-line.md` holds the `**Part of epic**` line and `epic.md` the epic as it stands.
- **Blocked slice** — `init=refused reason=blocked`: see `ferrule guide show gate1`.
- **Kept skeleton change** — write `milestone-change.md` and run `ferrule issue post {slug} --comment`: the milestone's description is updated in place with a changelog entry. Without milestones it posts `issue-comment.md` on the epic.
- **Split** — `ferrule issue post {slug} --split` files the split-off issue in the same milestone with `parent` set to the issue it came from. Without milestones it prepends `**Part of epic** #<e> · split from #<slice>` and names it in a one-line comment on the epic.
- Slices whose `Depends on` don't chain may run at the same time; gate 4's ID collision check covers them.
