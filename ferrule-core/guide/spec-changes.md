# spec-changes — How the spec settles after the brief

## Spec changes

The spec is drafted best effort and settles as the run learns:

| Phase | Who changes it |
|---|---|
| architecture | `fe-architect`, in `spec-diff.md`, local IDs |
| gate 2 | `fe-planner`, in `spec-diff.md`, then fixes IDs |
| after `s0` | `fe-implementer`, in the branch's `docs/specs/`, inside the stage commit |

Before `s0` the agents ask the user through their question batches; what they decided alone shows in the next stop's `Confirm:` block.

**After `s0`.** The implementer logs `spec-delta` on its card and reports `delta=<n>`. The stage's reviewer (full lane: the per-stage review; fast lane: a fresh `fe-reviewer` in scope `delta s<n>` whenever `delta>0`) classifies each change (`ferrule guide reviewer --section 'Spec changes'`):
- **detail** → listed on the verdict and the auto-continue line; no decision. Kept: `fe-author` writes `issue-comment.md`, posted with `ferrule issue file {slug} --comment` (`ferrule guide issue`).
- **meaningful** → a `Needs user` item: keep, revert, or split. Stage-stop rule 1 fires. The answer goes to the implementer.
  - **keep**, issue filed → resume `fe-author` for `issue-comment.md`, post it with `ferrule issue file {slug} --comment`, log `spec-keep=<IDs>` on the parent card. A kept architecture deviation → also a fresh `fe-architect` records it in `architecture.md` and its summary, no stop.
  - **revert** → the implementer reworks to the approved text, gauntlet, follow-up pass by the same reviewer.
  - **split** → `ferrule guide split`; the implementer narrows the entry.

Gate 3's reviewer checks every branch spec change was classified; gate 4's sync makes `spec-diff.md` the final spec, never one the orchestrator composes.

