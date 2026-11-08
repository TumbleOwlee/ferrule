Look up facts yourself; ask only decisions. Collect every decision you can state without knowing another's answer into one file. A decision that depends on one other decision of the same batch joins it, stated under the answer it assumes (`Assumes:`, that decision's recommended option); you ask it again in the next file only if the user answered that decision otherwise. One that depends on two or more answers waits for the next file. Rewritten whole each time. One block per decision:

```
## 1. <decision, one line, as a question>
Entry: **<ID>** — <the entry's current text in full>          ← whenever the decision touches a spec entry; one line per entry
Assumes: <n> = <option>                                        ← only for a decision that depends on decision <n> of this batch
Why you: <the decision criterion an agent must not settle alone — ownership, scope, a visible trade-off, a default>
Options:
- a) <option> (recommended: <≤ 1 line why>)
- b) <option>
Costs: a) <files / stages / spec impact> · b) <…>        ← only when options differ
Answer: a · b · other <text>
```

`Why you:` is mandatory. If you cannot fill it with a real criterion, the question is yours to answer — decide, append it as a `(chosen)` line to `decisions.md`, move on.
