# gate3 — The whole-branch review

## Gate 3 — review

Always a whole-branch review, whatever the stage tiers. Before proposing a PR, whole-branch pass — cross-stage bugs, spec drift across stages, scope creep:
- Fresh `fe-reviewer`, scope `branch`; each pass a fresh reviewer. Give it base ref, artifact dir, worktree path, all stage ids. Already run as the last code stage's review point (`ferrule guide show stage-review --section 'Per-stage review'`) and no code changed since → skip to gate 4. With `live-repo` set the live check runs before this review, after the last code stage is committed and pushed: `ferrule run live-check {slug} start`, the live check, `ferrule run live-check {slug} end`; `live-check=refused reason=open-stage` → commit and push the stage it names first.
- Orchestrator runs `ferrule ci gauntlet {slug}` (the full level; it logs `gauntlet=`) on the branch, forwards both status lines.
- The status line → `ferrule gate next {slug} gate3 …`: findings → the same implementer fixes, then a fresh branch review, never a follow-up pass here; findings needing a user decision are relayed as questions, never fixed unasked. It logs `review=<n>-open`, then `review=clean` or `review=<n>-resolved` (the PR status line reads it).
- Minors (`ferrule guide show verify`) are resolved before `fe-author` writes `pr.md`; then `ferrule pr push {slug}` (fast checks, `pushed=<sha>` on the parent card).
- **A `Needs user` item left:** full lane → `ferrule prompt render {slug} gate3`; fast lane → it rides gate 4's stop, answered by its letter, and an answer that changes code → fix, gauntlet, a fresh branch review, gate 4 again.
- **Clean:** no stop.
- **Board:** reviewer appends to `{run_dir}/review.md`, keyed to stage id, and rewrites `review.verdict.md`. Parent card → `inreview` when review starts.
