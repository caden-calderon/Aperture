# .context/ — Development Working Memory

> **This folder is part of the development workflow, not the submission.**
> For canonical documentation, see [`docs/`](../docs/DOCS_INDEX.md).

This directory contains session-level working memory used during active development with AI coding tools. It includes resume points, diagnostic prompts, investigation notes, and coding standards. Think of it as a developer's scratchpad that persists across sessions.

## Key Files

| File | Purpose |
|------|---------|
| `RESUME.md` | **Session entry point** — current state, what to read, next steps |
| `CODE_STANDARDS.md` | Coding conventions for Rust, Svelte 5, and testing |
| `AUDIT_PROMPT.md` | Security/quality audit prompt template |
| `FRONTEND_INVENTORY.md` | Frontend component and store inventory |

## Session Prompts

Files like `*-prompt.md` and `binary-zooming-alpaca.md` are kickoff prompts for AI coding sessions — they provide context after conversation compaction or session restart. These are artifacts of the development process, not documentation.

## Archive

Historical notes that are no longer actively referenced live in `.context/archive/`.

## Authority Rules

- If anything here conflicts with `docs/`, prefer `docs/` (canonical reference).
- If anything here conflicts with `dev/active/`, prefer the active phase docs (execution truth).
- `RESUME.md` is always the freshest snapshot of current development state.
