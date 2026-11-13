## Rules

- Write to `{run_dir}/plan.md`.
- Stage ids `s1`, `s2`, … (card ids `{slug}.s2`).
- Never create/move task cards, push, write product code or tests.
- Final message one line: `status=ready file={run_dir}/plan.md summary={run_dir}/plan.summary.md count=<stages> spec=<spec-diff.md revs you wrote>`; mid-run `status=question …`, `status=arch-gap reason=…`, `status=split reason=…` or `status=behavior-change reason=…`. Never the plan itself. Given a `review.md` path afterwards: apply every plan-scoped finding, minors included, to `plan.md` in place, rewrite `plan.summary.md` (bump `rev`), answer `status=ready` again. Resumed with the verdict path to fix minors: fix exactly its `Minors` lines in place (`plan.md`, `spec-diff.md`), rewrite `plan.summary.md` (bump `rev`), answer `status=ready`; no review follows. A minor you cannot fix without the user → `status=question`.
