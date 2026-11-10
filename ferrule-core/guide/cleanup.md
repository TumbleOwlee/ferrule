# cleanup — Ending a run

## Clean up

The run's last step, right after CI is green (or, for a run without a PR, once its work is pushed) — no approval stop: the issue and the PR hold everything that outlives the run, the metrics its cost. The human reviews and merges on the host, outside the workflow.

```sh
ferrule run cleanup {slug}
```

It refuses (exit 1, one line) while the worktree has uncommitted changes or commits its upstream lacks, or while the parent card's last `ci=` is not 0 and no later `ci-waived` line exists (an epic, a request run and a run without a PR whose issue is not open — none, or closed — need no CI result; its metrics line records `ci=-`); it also refuses a feature run without a PR while its issue is open (`issue-open`) or its state cannot be read (`issue-unreadable`), the latter going through only on the user's word: `ferrule board log {slug} issue-waived -- <why>`; else it records the run's metrics line (`ferrule run metrics`; a re-run replaces it), removes the worktree, deletes the local branch (the pushed one stays for the PR) and archives every card and `{run_dir}/` in the state database (never pruned; `ferrule run export` writes an archive back out), printing `cleanup=done archive=<id>`, with ` issue=<n>` appended when the run references an issue. The metrics line and the archive are written in one transaction before the worktree and branch are removed; if removing them fails (`worktree-remove-failed`, `branch-delete-failed`), fix the cause and rerun — it finishes only what is left and prints `cleanup=done archive=<id> resumed`, with no second archive. One notice line: `Done · {slug} · PR #<pr> ready · CI green · cleaned up · archived` for a run with a PR; for a run without a PR, `Done · {slug} · issue #<n> · cleaned up · archived` with an issue, `Done · {slug} · cleaned up · archived` without.

## Abandon

For a run the user gives up, run `ferrule run abandon {slug}` first and show its listing; run `ferrule run abandon {slug} --confirm` only on the user's word. It comments on and closes the PR, deletes the remote and local branch and the worktree, archives the cards as abandoned and leaves the issue open. A merged PR or an epic's parent is refused. After a failed step fix the cause and rerun. One notice line: `Abandoned · {slug} · issue #<n> open · archived` when the output carries `issue=<n>`, `Abandoned · {slug} · archived` without.
