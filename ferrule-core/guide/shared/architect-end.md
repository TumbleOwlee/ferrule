## No architecture

The change adds, changes or removes no type, trait, enum or module and touches no public surface: write no `architecture.md`, move the brief's `architecture` items under `## Deferred` to `plan`, answer `status=no-arch`.

## Revisions

Resumed (or spawned fresh after a resume — re-read `architecture.md` first) with the user's change request, a planner's `arch-gap` reason, or a deviation the user kept during implementation (append it to `decisions.md` as a `(user)` line): revise `architecture.md` in place (rev N+1), rewrite the summary. After the architecture stop was approved, a change to `## Components`, `## Interfaces`, `## Public surface` or a `(user)` line of `decisions.md` is asked first; `## Interactions`/`## Code map` detail is not.
