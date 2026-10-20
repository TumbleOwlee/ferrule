# author-late — The split issue and gate 4's sync, PR text and follow-ups

Read by `fe-author` only when resumed for a split or for gate 4; everything in `ferrule guide show author` still applies.

## Split issue — `split-issue.md`

```
# <title, plain language>

## Goal
<what the user gets, one or two lines>
<what happens today, one line>

## Found
- reproduction
  - <step>
  - <observed>
- cause
  - <fact>
- constraint
  - <what a fix needs, one fact per line>
- parent assumes
  - <what the parent's narrowed entry now states>
  - <what stays unfixed until this lands>

## Existing spec
- <ID>, <ID> — <what they say>
  - <gap, one line>
  - likely to change | cited
```

Existing IDs only, never a planned one. No issue number anywhere — the orchestrator appends `Split from #<issue>`.

## Gate 4 — sync, then `pr.md`

**Sync first.** `ferrule run diff {slug} --specs` is the run's final spec. Rewrite `spec-diff.md` to match it: one `## <ID>` section per entry the branch added, changed or dropped, headings keep their ` · was R<n>` suffix (the PR's Issue column reads it), scenarios kept, `## Deferred` removed; bump the rev, add `- rev N (<date>) — synced from the branch: <IDs that differ from the filed text> (fe-author)`, then one dated line per ID changed since filing, `- rev N (<date>) — <ID>: <what changed, plain words> (<user | review | impl>)`. Origin = the last `spec-delta <ID> … origin=` of `ferrule board events {slug} --event spec-delta`, or `review` for a change with no card line (an architect or planner edit after filing). `ferrule pr body full` renders them under `## Spec changes since filing`, quoting the user.

**Then `pr.md`**, full form. Inputs: `spec-diff.md`, `plan.md`, `review.md`, `review.verdict.md`, `gauntlet.log` (its coverage line), `ferrule run log {slug} --stat`. Line 1 = `issue.md`'s line 1. The orchestrator prepends the status line and appends scenarios, requirements, spec changes, architecture, decisions and the closing line; write none of them. No stage id, gate or agent name in prose; file, module and script names in backticks — only spans holding `.` or `/` are exempt, a bare stage id in backticks is still refused (`ferrule pr body` refuses otherwise, exit 3 `body=refused stage-ref=…`). Then `## Gate 4 — follow-ups`, and answer `status=ready file={run_dir}/pr.md follow-ups=<n>`.

```
# <title>

## Why
<what the user gets, one line>
<what it cost today, one line>

## Changes
- <package> <module> — <what changed> (<IDs it serves>)
- docs/specs/<area> — <n> entries added, <n> changed
- README — <what>                                            ← when touched

## Tests
<n> of <m> entries pinned
- <ID> — <exception: pinned by a real-view test | not unit-testable per docs/specs/README.md | …>

## Verification beyond the gauntlet
- <manual run, what was done and seen>
- not run: <what and who waived it> | none

```

`## Changes` per package module, never per file. `## Tests` lists exceptions only. `not run:` is always present and carries the verdict's `Not checked` line.

## Gate 4 — follow-ups

Things the run noticed outside its goal: `ferrule board events {slug} --event follow-up` and the `follow-up — ` lines of `review.md`. Merge duplicates; drop one an open issue already covers (`ferrule issue search --state open <keywords>`) or one the branch fixed after all. One `{run_dir}/follow-up-<k>.md` per remaining item, `k` from 1, in the `split-issue.md` shape without `## Existing spec` and `parent assumes`. No issue number anywhere — the orchestrator appends `Found in #<issue>`. None left → no file, `follow-ups=0`.
