# feedback — PR review threads back into the run

## PR feedback

The user reviews on the draft PR, as inline comments on the diff. Once `pr` is on the parent card, **before every approval stop** (per-stage, gate 3, gate 4) and whenever the user says to check the PR:

```sh
ferrule pr feedback route {slug}
```

It fetches the unresolved threads into `{run_dir}/pr-feedback.md` and prints `pr=<n> threads=<count> stages=<s1,s3|none>`. With threads it logs `feedback=<count>` on the parent card and moves the implicated stage cards → `inprogress` (a thread's `path:` names the file; the stage whose `files` list carries it owns it — unknown → the last stage).

- `threads=0` → proceed to the approval stop.
- `threads>0` → this is a review pass, handled like `review.md` findings with `count>0`: resume the implementer with the `pr-feedback.md` path. It fixes, fills each thread's `reply:` block, and answers its lane's status (`ferrule guide show stages --section 'Fast lane'`: `committed`; else `inreview`); then the usual review path on the touched stages (fast lane: gate 3's branch review). After `ferrule pr push {slug}`:

```sh
ferrule pr feedback reply {pr} {run_dir}/pr-feedback.md s<n> <sha>
```

posts every filled reply block (`<stage> <sha>`: the stage and the pushed head), resolving `fixed` threads and leaving `declined` ones open for the user. A comment asking for behavior the spec doesn't cover is a spec change the implementer makes (`ferrule guide show spec-changes`); the review classifies it. `status=blocked` (a comment is a question, or ambiguous) → relay `reason=` to the user as a question, never answer on the PR for them.

A thread the user resolves themselves, or one whose `reply:` they answer on the PR, drops out of the next fetch. Gate 4 does not run while `threads>0`.
