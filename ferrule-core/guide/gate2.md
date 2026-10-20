# gate2 — The plan, the tracking issue draft, approval

## Gate 2 — plan and tracking issue

Spawn `fe-planner` with: `{run_dir}/spec-diff.md` path (none for a non-behavior change), `{run_dir}/architecture.md` path (none on `no-arch` and when the architecture step was skipped), `arch=skipped` when it was, affected area(s), anything the user volunteered. Nothing else; the agent explores the repo itself, starting from the architecture's code map. It writes `plan.md` (shape: `ferrule guide show planner --section 'Output'`) and `plan.summary.md`, edits `spec-diff.md` and assigns its fixed IDs.

Every status line of the gate (planner, plan reviewer, the author's issue answer) → `ferrule gate next {slug} gate2 …`:
- Review rounds per `ferrule guide show verify`: fresh `fe-reviewer` scope `plan`; any finding, minors included, resumes the planner (`plan.prev.md` first); the same reviewer's follow-up.
- The issue draft: `fe-author` (`ferrule guide show issue`); a non-behavior run spawns it now with the goal verbatim, `plan.summary.md`, `{run_dir}/`. Then the stop or the fast lane's notice.
- `arch-gap` → resume `fe-architect` with the reason verbatim; its `ready` → resume the planner, no stop of its own. `split` → `ferrule guide show split`; `behavior-change` → `ferrule guide show principles --section 'Principles — implementation'`.
- The plan's `Stop` column (which stages stop for approval beyond the `ferrule guide show stage-review --section 'Stage stop'` rules) is settled at this stop: the user may tick or untick stages in the answer.
- **Stop:** `ferrule prompt render {slug} gate2 --issue <new | n>` (`plan.summary.md`, the verdict when findings survived two rounds, the brief when it moved; `issue.md` only as its `Issue:` line). Option 1 adds stages to the `Stop` column, option 2 switches the lane.
- **Notice:** `ferrule prompt render {slug} gate2-notice`, no answer awaited, then *On approval*.

<!-- done: gate2 -->
**On approval:** `ferrule gate approve {slug} gate2 --issue <new | n> [--stops s1,s3] [--lane <lane>] [--branch <type>/{slug}]` does, in order:
1. Files or rewrites the issue (`ferrule guide show issue`) and records `issue`; records `gate1` too while it is pending (fast lane without an architecture stop).
2. Adds the worktree `{worktree}` on a new branch off `{main}` (`branch=` names it; default `feat/{slug}`).
3. Records `gate2` (and `lane=`, for a non-behavior change with `no-arch`), moves the parent → `inprogress`.
4. Creates one stage card per row of `plan.md`'s `## Shared` stage table (`s0` only when the plan has one): `title`, `files` and `blocked-by` from the table, `stop: yes` for every stage `plan.summary.md`'s `Stop` column marks and every stage `stops=` names (the user's ticks at the stop). Stage ids match plan ids. It prints `approve=done gate=gate2 issue=<n> worktree=… stages=<count>`; `approve=refused reason=no-stages` (every `approve=refused` line ends in `hint=<fix>`) → the planner's table is missing or malformed: resume the planner.

Then spawn the implementer, its brief naming the lane (`ferrule guide show stages`).
