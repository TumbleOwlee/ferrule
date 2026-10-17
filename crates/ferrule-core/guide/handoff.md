# handoff — Agent status lines, files and summaries

## Agent hand-off

Every agent writes its full output to a file and ends its turn with a **status line** — nothing else.

```
status=<token> [file=<path>] [summary=<path>] [stage=s<n>] [issue=<n>] [reason=<one line>] [count=<n>] [delta=<n>] [user=<n>] [spec=<n>]
```

| Agent | `status` tokens | `file` | `summary` |
|---|---|---|---|
| spec-author | `question` (`count=` decisions) · `ready` (`count=` `R`+`E` entries at gate 1; `size=epic reason=` when the brief is too big for one run) · `no-diff` (`count=0`) · `reuse` (`issue=`) · `new` — both with `file=` `issue.md` | `{run_dir}/spec-diff.md`, `question.md`, `issue.md`, `issue-comment.md`, `split-issue.md`, `pr.md` | none — `ferrule prompt lines` quotes each file |
| spec-architect | `question` (`count=` decisions) · `ready` (`count=` new + changed components; `slices=` path when it wrote an epic) · `no-arch` · `split` (`reason=`) · `behavior-change` (`reason=`, non-behavior run) | `{run_dir}/architecture.md`, `question.md`, `slices.md` (epic); edits `spec-diff.md` | `{run_dir}/architecture.summary.md` |
| spec-planner | `question` (`count=` decisions) · `arch-gap` (`reason=`) · `split` (`reason=`) · `behavior-change` (`reason=`, non-behavior run) · `ready` (`count=` stages, `spec=` `spec-diff.md` revs it wrote) | `{run_dir}/plan.md`, `question.md`; edits `spec-diff.md` | `{run_dir}/plan.summary.md` |
| spec-implementer | `inreview` (`delta=`) · `committed` (`sha=`, `delta=`) — `delta=` counts spec changes · `question` (`count=` decisions) · `blocked` (`reason=`) | stage card (`stage=`), `question.md` | none — the card is the summary |
| spec-reviewer | `clean` · `findings` (`count=` blockers+majors, `user=` `Needs user` items, `stage=` list) · `blocked` | `{run_dir}/review.md` | `{run_dir}/review.verdict.md` |

`file` is lossless, for the next agent: as long as it must be. `summary` is for the user: decisions and open findings only, rewritten whole on every pass, capped at 25 lines / 2 KB (`ferrule prompt show` refuses a larger one and exits 3 — the agent trims it).

- `question` = a batch of decisions in `{run_dir}/question.md` (*Relayed question* (`ferrule guide question`)).
- An agent returning more than the status line is told to move the rest into its file and answer again.
- A status line naming a file the user must approve (`spec-diff.md`, `pr.md`, a `summary=` path) → the stop's `ferrule prompt render` call (*Approval prompt* (`ferrule guide approval`)): it opens the files outside the context (`ferrule prompt show`) and quotes only their `ferrule prompt lines` lines. Exit 3 = a summary over its cap: the agent that wrote it trims it, never show the full file instead.

At gate 2, a re-plan, a branch review, gate 4 and CI the status line goes to `ferrule run next {slug} <plan|replan|branch|pr|ci> <status line>`: it logs the typed event (`plan-review=`, `review=`, `ci=`, `replan=pre-authorized`) and prints `next=`, `run:` the command, `do:` what to do and `guide:` the topic to read; `ferrule run next --help` has the rules.

Reviewer scope tokens: `plan` (gate 2), `plan from s<n>` (*Re-plan* (`ferrule guide replan`)), `stage s<n>`, `delta s<n>` (one stage's spec changes, *Spec changes* (`ferrule guide spec-changes`)), `branch` (gate 3).
