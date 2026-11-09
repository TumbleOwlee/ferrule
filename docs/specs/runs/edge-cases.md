# Runs — edge cases

## Unpushed work

**RU-E-001** — A run without a PR still gets the uncommitted-changes and unpushed-commits refusals; the CI exemption does not cover unpushed work.

## Issue waiver

**RU-E-002** — A run whose card carries an `issue-waived` line is archived without reading its issue's state, even while the issue is open.

## Request runs

**RU-E-003** — A run started with `ferrule run init --request` is a `/fe-request` run even when its `--issue` is a slice of an epic; it is not a slice run.

## Epic milestones

**RU-E-004** — An issue in a milestone not made by ferrule, such as a release milestone, runs as a slice with that milestone's description as its only epic context.

**RU-E-005** — Two concurrent slice runs can overwrite each other's milestone description update; ferrule re-reads the description just before writing and the remaining window is accepted.

**RU-E-006** — Filing an epic whose title matches a closed milestone is refused and names that milestone.

**RU-E-007** — An issue carrying only the old `Part of epic` line and no milestone runs as a plain run.

**RU-E-008** — Ferrule never closes a milestone; the user closes it.

**RU-E-010** — A milestone description write GitHub rejects stops the filing, names GitHub's message and the text's length, and cuts nothing.

**RU-E-011** — A kept change whose old text is no longer in the milestone description is refused and leaves the description unchanged.

**RU-E-012** — A slice filing adopts an open issue of the same title only when that issue is already in the epic's milestone.

## Issue metadata

**RU-E-014** — An area outside the specs table gets no label and the filing continues.

**RU-E-015** — A follow-up whose draft names a milestone that is closed or absent at filing is refused, names that milestone and files nothing.

**RU-E-016** — A follow-up of a run that is not a slice run is filed in no milestone, whatever its draft names.

**RU-E-017** — A split-off that adopts an open issue already holding another parent keeps that parent.

## Blocked slices

**RU-E-009** — A blocker whose state cannot be read counts as open for RU-R-032.

**RU-E-013** — A run start that cannot read an issue's blockers at all is refused and starts no run.
