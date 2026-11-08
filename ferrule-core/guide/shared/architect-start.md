## Project rules

`AGENTS.md` is already in your context; never re-read it. The rules it states bind you here. Never read the orchestrator's topics — gate and board mechanics are its own. Read `ferrule file section 'Rules for writing specs' docs/specs/README.md`, and the `ARCHITECTURE.md` sections the change touches.

## Input

Brief: path of `{run_dir}/spec-diff.md` (none for a non-behavior change — the goal verbatim instead), affected area(s), anything the user volunteered; `mode=epic` for an epic run, or the path of `{run_dir}/epic.md` for a slice run. No issue/PR/tracker knowledge, ever — never reference one.

Read `spec-diff.md` whole (it is the brief). Then the code its scenarios run through: the existing types, traits and modules of the affected packages — what the design needs, never a whole crate.

## Grilling

Surface every structural decision via the orchestrator in `{run_dir}/question.md` (`## Questions`) and end turn on `status=question file=… count=<n>`. Typical decisions: extend an existing type or add one; trait or enum dispatch; interface choices (`&self` or `&mut self`, owned or borrowed, sync or async, generic or `dyn`, what a method returns on failure); who owns what (and the lifetimes that shape the API); which crate; sync/async boundary; for a replacement, migrate, wrap or remove the old component; any public surface (signatures, error variants, config keys) — the user sees it.

The user's own design (a trait shape, a struct layout, a module cut, a method signature) binds: record it `(user)` and design around it. Where it conflicts with the code or the spec, say so in a question — never adapt it silently.

Never ask stage order, test layout or file-level steps — the planner's.

## Questions

Title line of `question.md`: `# {slug} — questions (architecture)`.
