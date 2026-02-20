# Documentation Lifecycle

Last updated: 2026-02-19

This file defines where docs belong, which docs are authoritative, and how to archive stale notes.

## Authority Order
1. `docs/DOCS_INDEX.md`
2. `docs/*` stable references
3. `dev/active/<initiative>/{context,tasks,plan}.md`
4. `.context/RESUME.md`
5. `docs/archive/*` and `.context/archive/*` historical records

## Where Docs Belong
- `docs/`: durable product/architecture/operations reference.
- `dev/active/`: in-flight implementation tracking for the current initiative.
- `.context/`: session carry-over and temporary working memory.
- `docs/archive/`: retired durable docs kept for history.
- `.context/archive/`: retired session notes/prompts kept for traceability.

## Naming Rules
- Durable docs: descriptive kebab-case names (`repo-structure.md` style is acceptable if introduced later).
- Execution docs: fixed names `context.md`, `tasks.md`, `plan.md` inside initiative folder.
- Session notes: date-stamped where practical (`topic-YYYY-MM-DD.md`).
- Avoid whimsical names for new long-lived docs.

## Archive Rules
Move a doc to archive when any of the following is true:
- It is superseded by newer docs and no longer used for active decisions.
- It is a one-off exploration note with no current execution owner.
- It is prompt text for a completed tranche/pass.

Archive process:
1. Move file into the correct archive folder.
2. Update relevant index (`docs/archive/README.md` or `.context/archive/README.md`).
3. Update any active docs that referenced the old path.
4. If needed, leave a short pointer in the original index (`docs/DOCS_INDEX.md` or `.context/README.md`).

## Update Checklist Per Significant Pass
- Update `README.md` documentation links if navigation changed.
- Update `docs/DOCS_INDEX.md` when adding/removing canonical docs.
- Update `.context/RESUME.md` and active phase docs with the latest pass summary.
- Keep open issues and known limitations explicit in one place.

