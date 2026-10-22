# Specs

The authoritative description of what this project does, one directory per capability area. Code conforms to the spec, never the reverse. Written and kept by the spec-driven workflow (`ferrule guide show spec-changes`); the first areas are drafted by `/fe-bootstrap`.

## Areas

| Area | Directory | ID prefix |
|---|---|---|

## Per-area files

- `requirements.md` — what must be true, one entry per rule.
- `edge-cases.md` — deliberate limits and ugly-but-intended behavior, so nobody "fixes" them.
- `api-contract.md` — the public surface (flags, config keys, exported signatures, error variants), when the area has one.

## Rules for writing specs

These rules bind text in `docs/specs/`. A run's draft entries before they land (local `R<n>`/`E<n>` IDs in its `spec-diff.md`) may be coarser; they meet the rules once they land.

1. **No code pointers.** Never cite file:line or internal identifiers — a spec states what must be true, not where. Public, user-facing surface is spec and lives in the area's `api-contract.md`.
2. **ID format.** `<PREFIX>-R-nnn` for a requirement, `<PREFIX>-E-nnn` for an edge case; the prefix is the area's (table above). The two series are numbered independently per area. IDs are stable and append-only once on the main branch: a dropped ID is retired and its number never reused.
3. **Owner = the behavior, not the surface.** Shared behavior is stated once, in the area that owns it.
4. **Requirements are testable.** Normative indicative statements with observable outcomes. Good: "The client retries with exponential backoff bounded to 1s–30s." Bad: "The client is robust."
5. **Known gaps are specified, not hidden.** Intentional-but-ugly behavior goes into the area's `edge-cases.md` as a stated constraint.
6. **One entry, one physical line**, table rows included, so `grep -rn <ID> docs/specs/` returns the whole entry. Headings are unnumbered: `ferrule file section` matches heading text verbatim.
7. **Cite every ID a test pins, and only those.** The test's doc comment lists every requirement or edge-case ID its assertions directly verify, before the summary (`/// AB-R-012, AB-R-157 — …`).
8. **One entry, one rule.** One subject, one condition, one outcome. Can a single test pin the whole entry, and can one counterexample falsify it? If not, split it.
