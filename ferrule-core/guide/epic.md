# epic — Epic runs and slice runs

## Epic

A feature too big for one run: the author answered `size=epic`, or the architect's size guard tripped. The epic run settles the shared core and cuts the feature into slices; it writes no code and has no plan, worktree or PR. Each slice is later a normal run on its own issue.

1. **Brief** — gate 1 as usual; its stop approves `epic`. Or the architect's size guard turned a behavior run into one: `ferrule board set {slug} kind=epic lane=`, go to step 3; the epic stop approves the brief too when `gate1` is not yet a date (fast lane).
2. **Skeleton and slices** — spawn `fe-architect` with the brief and `mode=epic`. It writes `architecture.md` and its summary for the **skeleton** only (the core abstractions every slice builds on, with their `## Interfaces`) and `slices.md` (shape: `ferrule guide show architect-epic`). Questions relayed as usual; it answers `status=ready … slices={run_dir}/slices.md`.
3. **Epic issue draft** — `fe-author` drafts `issue.md` (`ferrule guide show issue`: existing issues searched, `reuse` or `new`).
4. **Epic stop** — `ferrule prompt render {slug} epic [--issue <n>]` (`<n>`: the author answered `reuse`); its `Confirm:` names every brief ID no slice or two slices claim. Approved → `ferrule gate approve {slug} epic` (records `arch`, and `gate1` while pending).
5. **On approval, in order** — no further stop:
   1. File or rewrite the epic issue: `ferrule issue post {slug} [--reuse <n>]` (`ferrule guide show issue`); it records `issue`.
   2. Per slice, in `slices.md` order: `ferrule issue post {slug} --slice <n>`. An open issue already titled exactly as the slice's is adopted, never filed twice; the number is appended to the parent card's `slices:` — a later slice's `Depends on` links it.
   3. `ferrule issue post {slug}` once more: an epic whose `issue` is set is rewritten — the one exception to `ferrule guide show issue`'s never-rewritten rule — so its `## Slices` list links every child.
   4. `ferrule run cleanup {slug}` — no worktree, no CI; it records the metrics; cards and artifacts are archived.
   5. One notice line: `Epic · {slug} · #<e> · slices #a, #b, … filed · start with /spec-feature on #a`.

The human closes the epic once its last slice merges.

### Slice run

`/spec-feature` on a child issue ("take issue #n"), a normal run otherwise.
- **Gate 1 board:** `ferrule run init {slug} --issue <n>` already recorded `kind: slice` and `epic: <e>`, stored the slice's `**Part of epic**` line (`slice-line.md`, which the issue body puts under the run's title) and the epic as it stands, its spec and comments (`epic.md`). Name `epic.md`'s path in every spawn brief of the run except the implementer's.
- **Kept skeleton change** — `ferrule issue post {slug} --comment` posts its `issue-comment.md` on the epic too.
- **Split** — `ferrule issue post {slug} --split` prepends `**Part of epic** #<e> · split from #<slice>` to a slice's split-off issue and names it in a one-line comment on the epic.
- Slices whose `Depends on` don't chain may run at the same time; gate 4's ID collision check covers them.
