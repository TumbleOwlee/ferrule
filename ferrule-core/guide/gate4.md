# gate4 — The pull request, follow-ups and CI

## Gate 4 — pull request

<!-- done: review=clean -->
- Gauntlet + gate 3 clean, `ferrule guide show feedback` at `threads=0`.
- `fe-author`, given the worktree path `{worktree}` and the run directory `{run_dir}`, syncs `spec-diff.md` from the branch's `docs/specs/`, writes `{run_dir}/pr.md` in **full form** and one `follow-up-<k>.md` per thing the run noticed outside its goal (implementer cards' `follow-up` lines, `review.md`'s `follow-up — ` lines; answers `follow-ups=<n>`) (content rules: `ferrule guide show author-late`). `ferrule pr body {slug} full` prints the body it makes; exit 3 (`body=refused reason=stage-ref stage-ref=… hint=…`) → resume `fe-author` to reword `pr.md`.
- The full body is the draft minus `## Stages` and `## Review here`, with its own status line, the `pr.md` prose in place of `## Why` and `## Scenarios covered` in place of `## Scenarios`; its spec changes come from the synced `spec-diff.md` Revisions lines, so the tick boxes and board state drop out and the remaining sections keep their order.

<!-- done: pr=ready -->
- **One stop**: the author's `ready` → `ferrule gate next {slug} gate4 …`, then `ferrule prompt render {slug} gate4` (`pr.md`, every `follow-up-<k>.md`, the branch verdict; the plan summary after a pre-authorized re-plan). `re-plan: <why>` → `ferrule guide show replan`. Option 1 narrows the follow-ups (none given: all are filed); `stop` holds for a manual run — never pre-empt it.

<!-- done: pr=ready -->
- **Approved** → `ferrule gate approve {slug} gate4`, then:

```sh
ferrule pr publish {slug} [--follow-ups <k>,<k> | --follow-ups none]
```

  It refuses (each line ends in `hint=<fix>`; `ferrule pr publish --help` lists them) without the `gate4` date the approval records (`reason=gate4`), while review threads are open (`reason=threads` → `ferrule guide show feedback`), when another run has landed the same fixed IDs meanwhile (`reason=collide ids=…` → resume the implementer to renumber them on the branch — spec, test citations, `spec-delta` log — then `ferrule pr push {slug}` and publish again) or when the body names a stage (exit 3, `reason=stage-ref` → resume `fe-author`). Otherwise it replaces the draft's title and body with the full ones (opening the PR when the run has none), marks it ready, logs `pr=ready`, files every kept follow-up with `Found in #<issue>` as its last line (an open issue with the same title is adopted), logs `follow-ups=[<n>, …]` and prints `publish=done pr=<n> follow-ups=<#a,#b|none>`. A dropped follow-up is never filed. A failed publish (the host's error, exit 1) is rerun as is: a PR already ready is not edited again, `pr=ready` is logged once and each follow-up filed (`posted=follow-up issue=<n> k=<k>`) is not filed twice.

<!-- done: ci=0 -->
- **CI** — `ferrule ci wait {slug}` waits for the PR's checks (the output in `{run_dir}/ci.log`), logs `ci=` and prints `ci=<code> [failed=<jobs>]`, the failed run's error lines, then the next step:
  - `ci=0` → `ferrule guide show cleanup`, which records the metrics.
  - otherwise → implicated stage card → `inprogress`, resume or spawn the implementer to fix, `ferrule pr push {slug}`, `ferrule ci wait {slug}` again. No stop unless a fix needs a user decision.
  - A failure the user rules unrelated to the run (e.g. also red on `{main}`), on the user's word only → `ferrule board log {slug} ci-waived -- <why>`, then `ferrule guide show cleanup`.
