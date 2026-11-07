# Runs — API contract

## ferrule run init flags

- `--request` — records the run as a `/fe-request` run (RU-R-001)

## ferrule run cleanup output

- `cleanup=done archive=<id> issue=<n>` — the run references issue `<n>` (RU-R-008)

## ferrule run cleanup refusals

- `cleanup=refused reason=issue-open issue=<n> hint=<fix>` (RU-R-003)
- `cleanup=refused reason=issue-unreadable issue=<n> detail="<host error>" hint=on the user's word only: ferrule board log <slug> issue-waived -- <why>` (RU-R-004)

## Card fields

- `named-issue` — the issue `ferrule run init --issue` named; set only by ferrule, `ferrule board set` refuses it (RU-R-006)
- `kind=request` — a `/fe-request` run, set by `ferrule run init --request` (RU-R-001)

## Card events

- `issue-waived` — parent-card event with a required reason (RU-R-005)
