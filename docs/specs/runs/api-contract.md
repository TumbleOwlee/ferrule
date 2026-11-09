# Runs — API contract

## ferrule run init flags

- `--request` — records the run as a `/fe-request` run (RU-R-001)
- `--start-blocked` — starts a slice run on an issue with open blockers, on the user's approval (RU-R-033)

## ferrule run init output

- `init=done slug=<slug> kind=slice milestone=<n> slice=<k>` — the issue is in milestone `<n>`, at rank `<k>` by issue number (RU-R-017)

## ferrule run init refusals

- `init=refused reason=blocked issue=<n> blockers=<list> [unreadable=<k>] hint=<fix>` — no card is created (RU-R-032)

## ferrule run cleanup output

- `cleanup=done archive=<id> issue=<n>` — the run references issue `<n>` (RU-R-008)

## ferrule run cleanup refusals

- `cleanup=refused reason=issue-open issue=<n> hint=<fix>` (RU-R-003)
- `cleanup=refused reason=issue-unreadable issue=<n> detail="<host error>" hint=on the user's word only: ferrule board log <slug> issue-waived -- <why>` (RU-R-004)

## Card fields

- `named-issue` — the issue `ferrule run init --issue` named; set only by ferrule, `ferrule board set` refuses it (RU-R-006)
- `kind=request` — a `/fe-request` run, set by `ferrule run init --request` (RU-R-001)
- `milestone` — the number of the run's milestone; set only by ferrule (RU-R-016)
- `slice` — a slice run's rank in its milestone; set only by ferrule (RU-R-017)

## Card events

- `issue-waived` — parent-card event with a required reason (RU-R-005)
- `blocked-start` — parent-card event naming the open blockers the user approved starting with (RU-R-034)

## ferrule issue post output

- `milestone=<n> action=created|adopted|edited|recorded` — an epic filed on GitHub (RU-R-013)

## ferrule issue post refusals

- `post=refused reason=milestone-closed milestone=<n> hint=<fix>` (RU-E-006, RU-E-015)
- `post=refused reason=milestone-stale hint=<fix>` (RU-E-011)
- `post=refused reason=bad-dependency value=<text> hint=<fix>`
- `post=refused reason=rejected length=<n> detail="<host error>" hint=<fix>` (RU-E-010)

## ferrule issue milestones

- `ferrule issue milestones` lists open milestones as `#<n> <title>`; `--view <n>` prints one description (RU-R-028, RU-R-039)

## Issue draft lines

- `Areas: <a>, <b>` — a line at the top of a split-off or follow-up draft body, and a slice's `Areas:` key in an epic's slices.md, naming the spec areas to label (RU-R-024)
- `Milestone: <exact title> | none` — a follow-up draft's milestone, read for a slice run only (RU-R-028, RU-R-039, RU-R-040, RU-E-015, RU-E-016)

## Run files

- `milestone-change.md` — a kept change on a slice run: a `Decision:` line, a `Summary:` line, then `@@ old`, `@@ new` and `@@ end` blocks of the text to replace in the milestone description (RU-R-020, RU-R-021)
