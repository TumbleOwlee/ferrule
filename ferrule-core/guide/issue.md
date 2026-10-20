# issue — Tracking issue: drafted, filed, commented

## Tracking issue

Drafted inside gate 2 once the plan review is clean, approved at gate 2's stop (fast lane: pre-authorized at the architecture stop), filed on approval.

Orchestrator searches existing issues (`gh issue list --state all --search …`) for the same goal and passes candidate numbers — plus the one the user named at gate 1 — to `fe-author`, which writes `{run_dir}/issue.md` (title + goal; shape: `ferrule guide show author`) and answers `status=reuse issue=<n> file=…` or `status=new file=…` in one turn. Reuse, never duplicate. On reuse the orchestrator stores the issue's original body once (unless gate 1 already did) with `ferrule issue request <n> > {run_dir}/request.md` — the body as filed, or, when that body is one this workflow assembled (it carries `## Scenarios`, `## Spec (rev` or an `Original request` block), the text inside its `<details>` block.

Filing — `ferrule gate approve {slug} gate2` does it; on its own:

```sh
ferrule issue post {slug}               # new; an open issue with the same title is adopted
ferrule issue post {slug} --reuse <n>   # reuse: title and body replaced
```

It assembles the body from the run's files (`ferrule issue body {slug}` prints it), records `issue` and prints `issue=<n> action=<created|adopted|edited|recorded>`; `recorded` means the card already named the issue and nothing was posted.

**After filing, the body is never rewritten** — it carries no status: the draft PR, linked to the issue, shows the progress. One exception: an epic's body is edited once, at `ferrule guide show epic` step 5.3, to link its slice issues.

Every requirement or scenario change kept after filing, detail or meaningful (`ferrule guide show spec-changes`), gets `issue-comment.md` from `fe-author`, which the orchestrator posts with no further approval:

```sh
ferrule issue post {slug} --comment     # a slice run: on the epic too
```

Exit 3 (`body=refused stage-ref=…`): the visible text names a stage or an agent; resume `fe-author` to reword `issue.md`, never edit the body by hand.

The PR link is never added by hand: the PR's `Closes #<n>` line makes GitHub cross-link issue and PR.
