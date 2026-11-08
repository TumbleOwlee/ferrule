## Output

`plan.md` is flat markdown, headed so `ferrule file section` pulls exactly one section — a stage-scoped reader (implementer, stage reviewer) never opens the whole file. Heading text is exact and stable once written: implementers and stage reviewers find a stage by its `## Stage s<n>:` prefix, and the orchestrator copies each title onto its card, so a heading renamed after approval disagrees with its card and the draft PR.

- `## Shared` — first section. **Stage table** (below), verification approach if uniform across stages, any code reference cited by 2+ stages. `ferrule gate approve {slug} gate2` creates the stage cards from the table alone.
- `## Stage s0: Land spec` — the first stage whenever `spec-diff.md` has entries to land: copy each `## <ID>` into its `docs/specs/<area>/` file (exact target file + insertion point per ID, contract-file rows likewise; the heading's ` · was R<n>` never lands), one commit, no code. Every other stage is `blocked-by: [s0]`. A `no-diff` or non-behavior plan has no `s0` — no empty stage.
- `## Stage s<n>: <title>` — one per stage, self-contained: numbered file-level steps, tests added, `files` touched, `blocked-by`, ID→test table (scenarios included: `S<n>` → the integration test or the Verification step that walks it), the `## Deferred` items owned by `implementation` this stage meets, **Verification** (how exercised beyond unit tests — unit tests alone, or the manual steps that exercise it — plus expected coverage impact), expected commits.

`plan.summary.md` — the user's file, written last, rewritten whole on every revision, **≤ 25 lines / 2 KB** (`ferrule prompt show` refuses more). What and where only, never how; decisions live in `decisions.md`. `Files` is a count with the crate modules in brackets, never full paths. `Pins` lists ID ranges with a count; every entry and every scenario of `spec-diff.md` appears in exactly one stage's `Pins`. `Spec since architecture` names the local IDs you added (`+`), dropped (`−`) or changed (`~`) and the fixed-ID count. `Stop` marks a stage that must stop for the user's approval even when clean: one that lands something later stages consume (a new trait method or type, a wire/config-schema change, a new error variant, a public-surface change, any stage with ≥ 2 dependants) — `yes — <why>`; every other stage `—`. The user settles the column at gate 2; the orchestrator copies it onto the stage cards. Blank line between blocks:

```
# {slug} — plan summary (rev N)

Decide: <n> stages, <s0 → s1 → s2 …>. <why this cut, one line>

Spec since architecture: <+R9 −R3 ~E2 | unchanged> · fixed IDs <n>

| Stage | What | Pins | Files | Stop |
|---|---|---|---|---|
| s0 | Land spec | all <n> entries | <n> (docs/specs/<area>) | — |   ← only when there is spec text to land
| s1 | <≤ 12 words> | UI-R-366..369 (4) | <n> (<package> <module>, <module>) | yes — <≤ 6 words why> |
| s2 | <≤ 12 words> | … | … | — |

Verification beyond unit tests:
- s3 — <how it is exercised>
  - <one step or one thing that must be seen per line>
  - cannot be waived without asking
- none

Out of scope, raise separately:
- <one line each, or none>

Open: <decision the user still owes, or none>
```

Existing-code references at the step. Code the architecture's `## Code map` holds is cited by its entry's anchor, `(code map: \`fn parse_frame\`)` — the implementer pulls that entry itself, so the excerpt exists once. Code you read beyond the map (or every reference of a `no-arch` plan) is quoted inline: the exact signature/pattern to match, verbatim where that removes ambiguity, anchored on a name or a quoted unique string (`fn parse_frame`, the `case Timeout:` arm). Never a line number (stale before the reviewer reads it), never a separate refs section. **The plan and the code-map entries it cites are the implementer's only codebase knowledge** (the implementer is a fresh spawn): a step that sends it back into the codebase is incomplete — expand now, or have the architect add the entry (`arch-gap`). Steps point to `## Shared` for a reference it holds (`3. use retry helper — see Shared`).

Stage table — the first table of `## Shared`, one row per stage in plan order, exactly these four columns:

```
| Stage | Title | Files | Blocked by |
|---|---|---|---|
| s0 | Land spec | docs/specs/<area>/requirements.md | — |
| s1 | Add backoff | src/client.rs, tests/client.rs | s0 |
| s2 | Show status | src/tab.rs | s1 |
```

- `Stage` is the stage id; `Title` the heading's text after `## Stage s<n>: ` (`## Stage title`), character for character.
- `Files` and `Blocked by` are comma-separated; `—` for none. No backticks, no links: a cell is copied onto the card as it stands.
- A row whose first cell is not `s<n>` is ignored, so a table that is not the stage table may follow it.

Stages run one at a time in plan order, so the table records what each stage consumes, for review and resume:
- stage depends on every stage producing what it consumes (type, module, fixture, config key)
- plan order respects every `blocked-by`

## Stage title

The stage's display name, also its card's `title:`: imperative, verb first (`Update README`); ≤ 3 words, ≤ 20 characters; letters, digits, hyphens, spaces only; unique within the plan; no stage id. s0, when present, is always `Land spec`.
