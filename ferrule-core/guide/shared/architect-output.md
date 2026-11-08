## Output

`architecture.md`, headed so `ferrule file section` pulls exactly one section; heading text exact and stable once written:

````
# {slug} — architecture (rev N)

## Summary
Goal: <spec-diff.md's Goal line, or the goal verbatim>
Shape: <one line>

## Components
| Component | Kind | Package › module | Change | Role |
|---|---|---|---|---|
| `<Name>` | struct · trait · enum · module · fn | <package> › <module> | new · changed · existing · removed | <≤ 10 words; longer when it names what a reviewer needs — key members, variants, signatures> |

Both `ferrule pr body` forms, the draft re-rendered on every card move and the gate 4 full body, put this table verbatim under the PR's `## Architecture`; the PR has no interfaces section, so a Role cell carries what a reviewer must know without the diff.

## Structure
```text
<ASCII diagram of the components: "owns", "uses", "impl" on the edges>
```

## Interfaces
### `<Name>` — new | changed
```
<signature-level sketch: trait methods, struct fields with types, enum variants — no bodies>
```

## Interactions
### S1 — <scenario title>
1. `<Type::method>` — <what it does, ≤ 10 words>

## Public surface
- `<signature | error variant | config key>` — <entry ID>
- none

## Fit with existing code
- `<existing type or module>` — <kept | changed: what | replaced by `<new>` | removed>
- ARCHITECTURE.md — <section to update | none>

## Code map
### `<fn, type or quoted unique string>` — <package> <file>
<what the plan needs from it, one line>
```
<the verbatim lines a plan step needs: signature, fields, variants, the arm to extend>
```

## Deferred
- <open point> — <plan | implementation>
````

- Decisions — every structure choice, asked or not — go to `decisions.md`, never into this file: `## Decision lines`.
- `## Interfaces`: one subsection per new or changed component of `## Components`, in the project's language, as code that would compile with bodies filled in: a trait with every method signature, a struct with every field and its type, an enum with every variant and its fields. A changed existing type shows only its new or changed members. No bodies, no private helpers, no derives unless they are part of the contract (`Serialize` on a config type).
- `## Interactions`: one subsection per scenario of `spec-diff.md` (a non-behavior change: per affected flow).
- `## Code map` is the code you read that the plan will need, so the planner never reads it again and the implementer pulls single entries by anchor (`ferrule file section --prefix '\`<anchor>\`'`) — an anchor never changes once the architecture stop is approved: one `###` entry per location the change touches or must match, anchored on a name or a quoted unique string (never a line number), with the exact lines copied verbatim — never paraphrased — at most 15 per entry. Whole section at most about 150 lines: a location that needs more gets the anchor and the one-line note only. Leave out what you read and ruled out.
- `## Deferred`: the brief's `plan`/`implementation` items plus your own.

`architecture.summary.md` — the user's file, written last, rewritten whole on every revision, **≤ 25 lines / 2 KB** (`ferrule prompt show` refuses more). Shape only — decisions live in `decisions.md`. Blank line between blocks:

```
# {slug} — architecture summary (rev N)

Decide: <n> new, <n> changed components in <packages>; <the shape, one line>

Interfaces: <n> new, <n> changed — sketches in architecture.md `## Interfaces`

| Component | Kind | Package | Change | Role |
|---|---|---|---|---|
| `<Name>` | trait | <package> | new | <≤ 8 words> |

Public surface:
- `<signature | error variant | config key>`
- none

Open: <decision the user still owes, or none>
```
