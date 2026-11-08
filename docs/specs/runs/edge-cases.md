# Runs — edge cases

## Unpushed work

**RU-E-001** — A run without a PR still gets the uncommitted-changes and unpushed-commits refusals; the CI exemption does not cover unpushed work.

## Issue waiver

**RU-E-002** — A run whose card carries an `issue-waived` line is archived without reading its issue's state, even while the issue is open.

## Request runs

**RU-E-003** — A run started with `ferrule run init --request` is a `/fe-request` run even when its `--issue` is a slice of an epic; it is not a slice run.
