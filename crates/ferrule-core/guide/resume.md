# resume — Resuming an interrupted run

## Resume an interrupted run

**Step 0.** `git pull --ff-only` in the main checkout before reading anything.

**No card, PR exists** (the user names the issue and its draft PR): `ferrule run rebuild <issue> <pr>`. `rebuild=done … next=s<n>` → the reconcile table below, then *Re-plan* (`ferrule guide replan`) from `next=` with a fresh planner (`plan.md` is never rebuilt; `next=none` → *Gate 3* (`ferrule guide gate3`)). `rebuild=refused reason=no-stages` → the PR is ready: `ferrule run ci`, then *Clean up* (`ferrule guide cleanup`). No PR → gate 1 with "take issue #n". Any other refusal → report it.

A card in `inprogress` or `inreview` (`ferrule card board`), no agent running = session died mid-run. Resume triggered by the user or `/spec-feature`, never automatic.

`kind: epic` with `issue` set → the epic stop was approved: continue with `ferrule issue file {slug} --slice <n>` at the first slice not yet in `slices:` (one filed before the crash is adopted, never filed twice). Otherwise: no worktree on the card → died during gate 1, architecture or gate 2, nothing on disk to reconcile — resume the conversation from `spec-diff.md`/`architecture.md`/`plan.md`'s last state (`arch` set → gate 2). `issue` set but no worktree → gate 2 was approved: rerun `ferrule run approve gate2 {slug}`; the recorded issue is kept (`action=recorded`) and only what is missing is done. Past gate 2 → table below. **Any resumed implementation spawns a fresh agent**, its brief naming the card's `lane`.

**Reconcile before acting.** Card = intent, git and the host = fact: `ferrule run reconcile {slug}` checks worktree, branch, every `commit=`, every `done` stage and the last `gauntlet=pass` against git, then the issue, the PR and its body on the host (`--local` skips those), printing `differ row=…` lines, then `reconcile=clean|differ … next=s<n>`; `ferrule run gauntlet {slug}` at the pass sha answers `cached` when the card told the truth. What a host row means:

| Row | Disagreement means |
|---|---|
| `issue #<n> is not open` | closed by hand: ask the user before going on |
| `issue #<n> has <c> comments for <k> kept spec changes` | a kept change was never announced: `ferrule issue file {slug} --comment` |
| `pr #<n> is merged` or `closed` | the run is over — `ferrule run ci {slug}`, then `ferrule guide cleanup` |
| `pr #<n> is a draft but pr=ready is logged` | publish died half-way: `ferrule pr publish {slug}` again |
| `pr #<n> body behind the board` | a move or a stage commit never rendered: the next `ferrule card move` or `ferrule pr push` brings it up to date |

No `pr` on the card: the draft never opened; the next `ferrule pr push {slug}` opens it.

Report differences first. Agree → resume. Disagree → stop and report.

A `replan from=` line with no later `replan=approved` or `replan=pre-authorized` on the parent card → the re-plan is pending: rerun *Re-plan* (`ferrule guide replan`) from step 2 with a fresh planner; `plan.prev.md` is already the snapshot.

Clean reconcile → resume only no-approval work (respawn the implementer for approved stages, CI wait and *Clean up* (`ferrule guide cleanup`) after `pr=ready`); halt at the first gate needing the user. Recorded `gate1`/`arch`/`gate2` approvals stay valid — the fast lane's pre-authorization too: `arch` set, `lane: fast`, no `gate2` → plan, then gate 2's *On approval* without a stop; `arch: skipped`, no `gate2` → plan, then gate 2's stop. A resumed run spawns every agent fresh (the architect, the author and the planner re-read their files).
