# lanes — Fast or full: how many approval stops a run makes

## Lane — fast or full

The lane sets how many approval stops the run makes. An epic run (`ferrule guide show epic`) has no lane: it stops at its brief and at the epic stop; each slice picks its own lane. Proposed by size when the first agent output of the run is ready; the user settles it at the run's first approval stop, recommended lane first.

Recommended default, by size:
- **fast** — ≤ 3 `R`/`E` entries in the brief, or `no-diff`; non-behavior change: `no-arch`, or ≤ 3 new or changed components (architecture `count=`). At most 1 entry, or `no-diff`: no architecture step (`arch: skipped`), the planner starts from the brief.
- **full** — anything bigger.

| | fast | full |
|---|---|---|
| Brief (gate 1) | no stop of its own — approved at the architecture stop, or at gate 2's stop without one | own stop, settles the lane |
| Architecture | one stop: brief + architecture + lane; approving pre-authorizes gate 2's *On approval*; no architecture step at ≤ 1 entry | own stop |
| Gate 2 | a notice unless `ferrule gate next` stops it (`ferrule guide show gate2`); no architecture step: one stop, brief + plan + issue | plan summary + issue draft |
| Stages | `ferrule guide show stages --section 'Fast lane'` | `ferrule guide show stages --section 'Full lane'`, stops per `ferrule guide show stage-review --section 'Stage stop'` |
| End | one stop, `ferrule guide show gate4`, showing the branch verdict and any `Needs user` item it left | `ferrule guide show gate4` stop with the branch verdict; a separate `ferrule guide show gate3` stop only for a `Needs user` item |

No architecture step and the planner answers `arch-gap`: spawn `fe-architect` fresh with the brief and the reason; its `ready` → the architecture stop (approves the brief), then resume the planner.

Recommended full, user picks fast at the brief stop: the architecture stop is fast's. The fast default for a plan is at most 2 stages besides `s0` (`ferrule guide show gate2`). The user may switch lanes at any later stop; a lane never switches silently.
