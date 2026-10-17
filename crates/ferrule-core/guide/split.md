# split — Splitting a change off into its own issue

## Split

A change needed only for something outside the goal; the user's choice at a question or a `Needs user` item. Before `s0` the raising agent (architect, planner) has already narrowed the entry when it answers `status=split reason=…`; after `s0` the reviewer only raised it, and the implementer narrows it last (below). Orchestrator resumes `spec-author` with the reason verbatim (after `s0`: the verdict item and the `review.md` path); it writes `split-issue.md`. Orchestrator files it with `ferrule issue file {slug} --split` — at once if the tracking issue is filed, else right after `ferrule run approve gate2`; it appends `Split from #<issue>` as the body's last line (a slice run: see `ferrule guide epic`) and the number to the parent card's `split:`. Then post one notice line `Split · {slug} · <IDs> · #<m> filed`, then resumes the architect or planner that raised it — after `s0`, the implementer with `split <ID>: narrow it`, gauntlet, the usual review. The new issue is its own later run.

**Fold in** — the user's explicit override of a split: the raising agent rewrites the `Goal:` line to the widened scope (after `s0`: it is a meaningful change, kept).
