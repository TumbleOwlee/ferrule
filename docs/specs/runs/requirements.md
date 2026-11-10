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

## Epic milestones

**RU-R-013** — On GitHub, an epic run files one milestone titled as the epic, and no epic issue.

**RU-R-014** — The milestone description holds the epic's brief and skeleton architecture.

**RU-R-015** — The milestone description lists no slices; the issues in the milestone are its slices.

**RU-R-016** — Every slice issue is filed in its epic's milestone.

**RU-R-017** — A run started on an issue in any milestone is a slice run of that milestone, except as RU-E-003 states.

**RU-R-018** — A slice run's context holds its milestone's description.

**RU-R-019** — An epic run started on an existing issue (RU-R-006) closes that issue with a comment linking the milestone and leaves its title and body unchanged, so RU-R-003 does not refuse its cleanup.

**RU-R-020** — A spec change kept by a slice run updates the milestone description in place.

**RU-R-021** — The milestone description ends in a changelog section with one compact entry per kept change or split-off: UTC timestamp, author, decision and a summary of the applied change.

## Issue metadata

**RU-R-022** — An issue ferrule files carries no line or title prefix stating its epic, its slice position or what it depends on.

**RU-R-023** — A slice that depends on another slice is filed with a `blocked by` relationship to that slice's issue.

**RU-R-024** — An issue ferrule files carries one label `area:<directory>` for each spec area it affects, `<directory>` being the area's directory under `docs/specs/`.

**RU-R-025** — A split-off issue has the issue it came from as its `parent` relationship, except as RU-E-017 states, and carries no `Split from` line.

**RU-R-026** — A split-off issue of a slice run is filed in that slice's milestone.

**RU-R-027** — A follow-up issue has a `relates to` relationship to the issue it came from and carries no `Found in` line.

**RU-R-028** — A follow-up of a slice run is filed in the run's milestone when its context fits that milestone.

**RU-R-039** — A follow-up of a slice run whose context does not fit the run's milestone is filed in another open milestone whose context fits it.

**RU-R-040** — A follow-up of a slice run whose context fits no open milestone is filed in no milestone.

**RU-R-029** — The gate 4 stop shows the milestone proposed for each follow-up, or that it has none.

**RU-R-030** — A rerun after a failed milestone, label or relationship write performs only the missing writes.

**RU-R-036** — A kept spec change that adds an area to the run's brief adds that area's label to the run's issue and removes none.

**RU-R-041** — A label ferrule creates in the repository gets a random color.

**RU-R-042** — A label the repository already has is left as it is.

**RU-R-037** — A rerun of an epic filing adopts an open milestone of the same title.

## Blocked slices

**RU-R-032** — A slice run starting on an issue with an open `blocked by` issue first asks the user, naming each open blocker.

**RU-R-033** — A slice run on a blocked issue starts only after the user approves.

**RU-R-034** — The user's approval to start a blocked slice is recorded on the run's card.

**RU-R-038** — Resuming a run whose blocked start the user approved does not ask again.

## Documentation

**RU-R-010** — The cleanup guide states the CI exemptions of RU-R-001 and RU-R-002 beside the epic's.

**RU-R-011** — The `ferrule run cleanup` help states the CI exemptions of RU-R-001 and RU-R-002 and the refusals of RU-R-003 and RU-R-004.

**RU-R-031** — The epic guide states that a GitHub epic is a milestone and that slices are the issues in it.

**RU-R-035** — The epic guide's closing notice of an epic filed as a milestone reads `Epic · <slug> · milestone #<m> · slices #a, #b, … filed · start with /fe-feature on #a`.

## Live check

**RU-R-043** — `ferrule run live-check <slug> start` opens the run's live check.

**RU-R-044** — `ferrule run live-check <slug> end` closes the run's live check.
