# cleanup — Ending a run

## Clean up

The run's last step, right after CI is green — no approval stop: the issue and the PR hold everything that outlives the run, the metrics its cost. The human reviews and merges on the host, outside the workflow.

```sh
ferrule run cleanup {slug}
```

It refuses (exit 1, one line) while the worktree has uncommitted changes or commits its upstream lacks, or while the parent card's last `ci=` is not 0 and no later `ci-waived` line exists (an epic is exempt); else it records the run's metrics line (`ferrule run metrics`; a re-run replaces it), removes the worktree, deletes the local branch (the pushed one stays for the PR) and archives every card and `{run_dir}/` in the state database (never pruned; `ferrule run export` writes an archive back out), printing `cleanup=done archive=<id>`. The metrics line and the archive are written in one transaction before the worktree and branch are removed; if removing them fails (`worktree-remove-failed`, `branch-delete-failed`), fix the cause and rerun — it finishes only what is left and prints `cleanup=done archive=<id> resumed`, with no second archive. One notice line: `Done · {slug} · PR #<pr> ready · CI green · cleaned up · archived`.
