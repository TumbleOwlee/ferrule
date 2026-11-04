# Gates

## Stage gate

**GA-R-001** — `ferrule gate approve <slug> stage` is approved while the parent card is `inprogress` or `inreview`.

**GA-R-002** — `ferrule gate approve <slug> stage` is refused with `reason=state` while the parent card is in any other state.

**GA-R-003** — The `ferrule gate approve` help lists `inprogress` and `inreview` as the parent states of the stage gate.

**GA-R-004** — The stage-review guide directs the last code stage's approved stage stop to commit and push the stage before gate 4.
