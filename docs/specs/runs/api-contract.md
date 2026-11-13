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
- `cleanup=refused reason=no-ci hint=record CI first: ferrule ci wait <slug>; on the user's word only: ferrule board log <slug> ci-waived -- <why>` (RU-R-061, RU-R-067)

## ferrule run preflight

- `preflight=ok gh=<login>` (RU-R-072)
- `preflight=ok gh=skipped host=<host>` (RU-E-037)
- `preflight=refused reason=gh-login hint=run gh auth login, then start again` — exit 1 on refused (RU-E-035)
- `preflight=refused reason=gh-unknown detail="<error>" hint=<fix>` — exit 1 on refused (RU-E-036)

## ferrule run live-check

- `ferrule run live-check <slug> start|end` — both take the run's slug; `start` refuses with `reason=no-repo` (RU-R-043, RU-R-044, RU-E-019)
- `start` also refuses with `live-check=refused reason=open-stage stage=s<n> hint=<fix>`, naming the first stage with no logged commit (RU-E-038)

## ferrule run abandon

- `ferrule run abandon <slug> [--confirm]` (RU-R-045–RU-R-054)

## ferrule run abandon output

- `abandon=plan` followed by one line per effect: `uncommitted <path>`, `unpushed <sha> <subject>`, `close-pr <n>`, `delete-remote-branch <branch>`, `delete-branch <branch>`, `remove-worktree <path>`, then `hint=on the user's word only: ferrule run abandon <slug> --confirm` (RU-R-045, RU-R-046, RU-E-020, RU-R-059, RU-R-060, RU-E-030, RU-E-031, RU-E-032)
- `abandon=done archive=<id>`, with ` issue=<n>` when the run references an issue and ` resumed` when an earlier abandon archived the run (RU-R-047, RU-R-056, RU-E-021)
- `abandon=already archive=<id>` — the run is fully abandoned, exit 0 (RU-E-026)

## ferrule run abandon refusals

- `abandon=refused reason=card` (RU-E-029), `reason=merged pr=<n> hint=…ferrule run cleanup…` (RU-E-023), `reason=epic` (RU-E-024), `reason=pr-unreadable detail="<host error>"` (RU-E-027), `reason=remote-unreadable branch=<branch>` (RU-E-028)
- `abandon=refused reason=close-pr-failed`, `reason=remote-delete-failed`, `reason=worktree-remove-failed`, `reason=branch-delete-failed`, each with ` detail="<error>"`; a rerun performs only the steps still missing (RU-E-021)

## ferrule run metrics output

- `kind=<plain|epic|slice|request>` in place of `lane=`, `reviews=plan:<n>,stage:<n>,batch:<n>,branch:<n>` and `time=brief:<m>,arch:<m>,plan:<m>,stages:<m>,branch:<m>,pr:<m>` in whole minutes, `-` for a phase the run did not have (RU-R-076, RU-R-074, RU-R-075)

## Card fields

- `named-issue` — the issue `ferrule run init --issue` named; set only by ferrule, `ferrule board set` refuses it (RU-R-006)
- `kind=request` — a `/fe-request` run, set by `ferrule run init --request` (RU-R-001)
- `milestone` — the number of the run's milestone; set only by ferrule (RU-R-016)
- `slice` — a slice run's rank in its milestone; set only by ferrule (RU-R-017)
- `live-repo` — the repository named at gate 2; set only by ferrule, `ferrule board set` refuses a value (GA-R-062, RU-E-018)
- `plan-stop: yes` — set by `ferrule gate approve <slug> arch --plan-stop`; only ferrule sets it (GA-R-129)

## Card events

- `issue-waived` — parent-card event with a required reason (RU-R-005)
- `ci-waived` — parent-card event with a required reason (RU-R-063, RU-R-064)
- `blocked-start` — parent-card event naming the open blockers the user approved starting with (RU-R-034)
- `live-check=open` and `live-check=closed` — parent-card events only ferrule logs (RU-R-043, RU-R-044)
- `abandoned` — parent-card event only ferrule logs (RU-R-047)
- `approved=<gate>` — run-card event logged by `ferrule gate approve` (RU-R-075)

## ferrule issue post output

- `milestone=<n> action=created|adopted|edited|recorded` — an epic filed on GitHub (RU-R-013)
- `milestone=<n> action=edited|recorded` — printed after `issue=<n> action=commented|recorded` by a comment post on a slice run in milestone `<n>`: `edited` the kept change was applied, `recorded` its changelog entry was already there (RU-R-020)

## ferrule issue post refusals

- `post=refused reason=milestone-closed milestone=<n> hint=<fix>` (RU-E-006, RU-E-015)
- `post=refused reason=milestone-stale hint=<fix>` (RU-E-011)
- `post=refused reason=bad-dependency value=<text> hint=<fix>`
- `post=refused reason=rejected length=<n> detail="<host error>" hint=<fix>` (RU-E-010)
- `post=refused reason=missing file=milestone-change.md hint=<fix>` — nothing is posted (RU-E-034)

## ferrule issue milestones

- `ferrule issue milestones` lists open milestones as `#<n> <title>`; `--view <n>` prints one description (RU-R-028, RU-R-039)

## Issue draft lines

- `Areas: <a>, <b>` — a line at the top of a split-off or follow-up draft body, and a slice's `Areas:` key in an epic's slices.md, naming the spec areas to label (RU-R-024)
- `Milestone: <exact title> | none` — a follow-up draft's milestone, read for a slice run only (RU-R-028, RU-R-039, RU-R-040, RU-E-015, RU-E-016)

## Run files

- `milestone-change.md` — a kept change on a slice run: a `Decision:` line, a `Summary:` line, then `@@ old`, `@@ new` and `@@ end` blocks of the text to replace in the milestone description (RU-R-020, RU-R-021)
