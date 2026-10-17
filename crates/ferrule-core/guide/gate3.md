# gate3 — The whole-branch review

## Gate 3 — review

Before proposing a PR, whole-branch pass — cross-stage bugs, spec drift across stages, scope creep:
- Fresh `spec-reviewer`, scope `branch`. Give it base ref, artifact dir, worktree path, all stage ids. Already run as the last code stage's review (*Per-stage review* (`ferrule guide stage-review`)) and no code changed since → skip to gate 4.
- Orchestrator runs `ferrule run gauntlet {slug}` on the branch, forwards both status lines.
- The status line → `ferrule run next {slug} branch …`: findings → the same implementer fixes, then a fresh branch review, never a follow-up pass here; findings needing a user decision are relayed as questions, never fixed unasked. It logs `review=<n>-open`, then `review=clean` or `review=<n>-resolved` (the PR status line reads it).
- Minors (*Verify before an approval stop* (`ferrule guide verify`)) are resolved before `spec-author` writes `pr.md`; then `ferrule pr push {slug}` (gauntlet, `gauntlet=pass sha=<sha>` on the parent card).
- **A `Needs user` item left:** full lane → `ferrule prompt render gate3 {slug}`; fast lane → it rides gate 4's stop, answered by its letter, and an answer that changes code → fix, gauntlet, a fresh branch review, gate 4 again.
- **Clean:** no stop.
- **Board:** reviewer appends to `{run_dir}/review.md`, keyed to stage id, and rewrites `review.verdict.md`. Parent card → `inreview` when review starts.
