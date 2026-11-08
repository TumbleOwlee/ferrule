# Runs

## CI exemption

**RU-R-001** — `ferrule run cleanup` applies no CI check to a `/fe-request` run.

**RU-R-002** — `ferrule run cleanup` applies no CI check to a `/fe-feature` run that opened no PR and references no open issue.

**RU-R-012** — The metrics line of a run `ferrule run cleanup` archives without a CI check records `ci=-`.

## Open issue refusal

**RU-R-003** — `ferrule run cleanup` refuses a `/fe-feature` run that opened no PR while its referenced issue is open.

**RU-R-004** — `ferrule run cleanup` refuses a `/fe-feature` run that opened no PR when its referenced issue's state cannot be read, with a hint that it goes through on the user's approval.

**RU-R-005** — Once the run's card carries an `issue-waived` line, `ferrule run cleanup` of a run RU-R-004 refused goes through without reading the issue's state.

## Issue reference

**RU-R-006** — A `/fe-feature` run started with an issue references that issue.

**RU-R-007** — A `/fe-feature` run that files its tracking issue references the filed issue.

## Closing notice

**RU-R-008** — The closing notice of a run without a PR that references an issue reads `Done · <slug> · issue #<n> · cleaned up · archived`.

**RU-R-009** — The closing notice of a run without a PR that references no issue reads `Done · <slug> · cleaned up · archived`.

## Documentation

**RU-R-010** — The cleanup guide states the CI exemptions of RU-R-001 and RU-R-002 beside the epic's.

**RU-R-011** — The `ferrule run cleanup` help states the CI exemptions of RU-R-001 and RU-R-002 and the refusals of RU-R-003 and RU-R-004.
