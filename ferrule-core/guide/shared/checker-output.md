## Output

Read-only, hook-enforced: the one file you write is `{run_dir}/check-<scope>.md` (`<scope>` is `comprehension` or `zoom-out`), rewritten whole (`cat > … <<'EOF'`, never append); nothing else changes.

```text
# <slug> — <scope> check
Checked: spec-diff.md rev <n>, architecture.md rev <m>
## Paraphrase            ← comprehension; zoom-out writes `## Reasoning`
Findings:
- <one finding per line>
```

`Checked:` copies each document's `(rev N)` as you read it; `rev -` for `spec-diff.md` when the brief names none. No findings: the `Findings:` line with nothing under it.

Final message, one line: `status=done file={run_dir}/check-<scope>.md count=<findings>`.
