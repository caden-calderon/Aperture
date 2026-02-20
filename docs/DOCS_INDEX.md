# Documentation Index

Last updated: 2026-02-19

This is the canonical navigation map for documentation in this repo.
If documentation conflicts, follow this file first, then `docs/DOC_LIFECYCLE.md`.

## Start Here (Fresh Context)
1. `README.md`
2. `docs/DOCS_INDEX.md`
3. `docs/HACKATHON_SUBMISSION.md`
4. `.context/RESUME.md`
5. `dev/active/phase-4-compression-readiness/context.md`
6. `dev/active/phase-4-compression-readiness/tasks.md`
7. `dev/active/phase-4-compression-readiness/plan.md`

## Documentation Authority
- `docs/`: stable reference docs intended to remain valid over time.
- `dev/active/`: execution artifacts (plans, tasks, session context) by initiative.
- `.context/`: working-memory artifacts for iterative sessions; can include historical or superseded notes.
- `docs/archive/`, `.context/archive/`: historical reference only, never source of truth.

## Canonical Reference Docs (`docs/`)
- `docs/ARCHITECTURE.md`: high-level architecture and runtime boundaries.
- `docs/INTEGRATION.md`: Tauri IPC/events/frontend-backend integration reference.
- `docs/PROVIDER_CAPABILITY_MATRIX.md`: provider/runtime capability matrix.
- `docs/SECURITY_BASELINE.md`: security constraints and hardening requirements.
- `docs/REPO_STRUCTURE.md`: repository layout and ownership boundaries.
- `docs/DOC_LIFECYCLE.md`: doc location, naming, archival, and update rules.
- `docs/HACKATHON_SUBMISSION.md`: concise project overview and demo/readiness snapshot.

## Active Execution Docs (`dev/active/`)
- Active phase now: `dev/active/phase-4-compression-readiness/`.
- Track-local source of truth is always the trio:
  - `context.md`
  - `tasks.md`
  - `plan.md`
- Older tracks under `dev/active/` are historical references unless explicitly reactivated.

## Session Memory Docs (`.context/`)
- `.context/RESUME.md` is the only required entrypoint for context carry-over.
- Other `.context/*.md` files are working notes, analyses, or historical prompts.
- Historical notes are moved under `.context/archive/`.
- See `.context/README.md` for classification and status of each file.

## Archive Docs
- `docs/archive/README.md`: index of archived product/reference docs.
- `.context/archive/README.md`: index of archived session working notes.

## Naming and Organization Rules
- New durable docs belong in `docs/`.
- New execution/phase docs belong in `dev/active/<initiative>/`.
- `.context/` should not be used as permanent architecture/API reference.
- Prefer descriptive kebab-case names (avoid whimsical names for long-lived docs).
- When a note is no longer active, move it to an archive folder and keep an index entry.
