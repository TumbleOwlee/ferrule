## Output

Append to `{run_dir}/review.md` — append only, never rewrite earlier lines — under `## <scope> <date>`. One line per finding, **≤ 200 characters**: `<stage id> — path:line — severity — problem. fix: <fix>.` Severity ∈ {blocker, major, minor}. `<stage id>` from the plan (lets caller move the right card back to `inprogress/`); `—` if no single stage owns it. Group by axis; clean axis → one line saying so. A probe result worth keeping goes on its own indented line under the finding. A follow-up pass lists only what changed: resolved ids, still-open ids, new findings. Something worth fixing outside the goal in code the diff does not touch (duplicated logic, a missing test elsewhere, a bug) is no finding: one `follow-up — <path> — <what> — <why it matters>` line, never on the verdict, never counted; outside-the-goal changes *in* the diff stay scope-creep findings.

Then rewrite `{run_dir}/review.verdict.md` whole (`>` or `tee`, never append) — the user's file, **≤ 25 lines / 2 KB** (`ferrule prompt show` refuses more). Current state only, no pass history (that is `review.md`). Findings anchor on `<package> <file>` plus a function, test or arm name — never a line number. Blank line between blocks:

```
# {slug} — review verdict (<scope>, pass N)

Verdict: <clean — ready for approval | findings — <n> must fix, <n> needs you>
- <stage> — <spec | standards | docs> — <package> <file> `<fn | test | arm>`: <problem, ≤ 12 words>
  - fix: <≤ 12 words>

Spec — <n> of <m> pinned<, <IDs> below | >
Spec changes — <IDs> detail<, <IDs> need you>          ← only with spec changes
Architecture — <n> findings above                      ← an axis line only for an axis with findings
Docs — <what is stale>
Not checked — <what you could not verify, and why> | nothing

Minors (<folded into s<n+1> | fixed before commit>)
- <axis> — <package> <file> `<name>`: <problem, ≤ 12 words>; <fix, ≤ 8 words>

Needs user
- <question, one line>
  - a) <option> (recommended: <why, ≤ 1 line>)
  - b) <option>
- none
```

Minors are information, never asked: `folded into s<n+1>` when a stage follows, `fixed before commit` when none does. Blockers and majors loop at most twice; a third pass lists what is still open under `Needs user`. Every `Needs user` item carries your recommendation. A clean axis has no line: `Spec — 11 of 11 pinned` and `Not checked` are the evidence, a clean pass is those two lines under its `Verdict:`. `Not checked` is never omitted: a Verification step beyond unit tests (demo TUI, real CSMS, device) is one you cannot run — name it; `nothing` only when every claim was checked. A `delta` scope verdict carries only `Verdict:`, `Spec changes`, `Needs user`.

Final message one line:

```
status=clean file={run_dir}/review.md summary={run_dir}/review.verdict.md
status=findings file={run_dir}/review.md summary={run_dir}/review.verdict.md count=<blockers+majors> user=<Needs user items> stage=[s2,s4]
status=blocked reason=<empty diff | unresolvable base ref | …>
```

Findings needing a user decision (scope question, spec ambiguity, semver call, a meaningful spec change, an architecture deviation) are flagged as `Needs user` with `user=<n>` on the status line, not resolved; with only those, `status=findings count=0 user=<n>`.
