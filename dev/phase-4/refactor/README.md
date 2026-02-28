# Phase 4 Refactor Docs Index

This folder splits the old `refactor-plan.md` into focused files so exploration sessions
load only what they need.

## Read Order (Per Session)
1. `dev/phase-4/context.md` (protocol + current progress)
2. `dev/phase-4/refactor/README.md` (this file)
3. Your active audit file only:
   - Claude: `dev/phase-4/refactor/audit-claude.md`
   - Codex: `dev/phase-4/refactor/audit-codex.md`
4. `dev/phase-4/refactor/uncertain-findings.md` only when logging/checking uncertain items
5. `dev/phase-4/refactor/session-log.md` only when closing a session

## File Map
- `overview.md` — master plan and phase definitions
- `audit-claude.md` — Claude file-by-file audit table
- `audit-codex.md` — Codex file-by-file audit table
- `uncertain-findings.md` — shared unresolved observations
- `session-log.md` — Claude/Codex session history

## Bug Logging
- Primary source: audit tables (`audit-claude.md`, `audit-codex.md`).
- External summary index: `~/.claude/projects/-home-caden-projects-Aperture/memory/bugs.md`.

## Context Hygiene Rule
- Do not re-read historical audit rows or past sessions each turn.
- Read only the next unread files from `context.md`, append new rows, and move on.
- Re-open old rows only for explicit reconciliation or bug confirmation.
