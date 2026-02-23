# Round 11 — Regression: 3rd Plan Round Fails (2026-02-21)

## Context
Hackathon demo filming session. Running Claude Code through Aperture proxy. All prior fixes (Option B, diagnostic tracing, MCP retry) in place.

## Observations
- Round 1: Committed plan (4 blocks archived). Fired correctly on subsequent turns.
- Round 2: Committed plan (5 blocks archived). Fired correctly. Accumulated with round 1 (4+5 = 9 blocks stripped per turn).
- Round 3: Committed plan (13 blocks archived). MCP returned success. **NEVER fired.**
  - Breadcrumb never showed round 3 additions
  - `/context` kept growing (conversation overhead outpaced the 9-block archival from rounds 1+2)
  - Confirms the 13 blocks from round 3 were NOT being removed

## Analysis
Pending JSONL log analysis. R9-DIAG traces should show:
- Whether `add_persistent_archives_for_session()` was called for round 3
- What the persistent count was after the 3rd commit
- Whether session ID matched across the 3rd commit path

## Hypotheses
- H3: `add_persistent_archives_for_session()` replaces rather than merges on 3rd call
- H4: Block IDs in 3rd plan stale (already removed from session store by rounds 1+2)
- H5: Session ID drift after multiple plan cycles
- H6: Capacity/collection issue at 3+ rounds

## Log Location
`~/.claude/projects/-home-caden-projects-Aperture/` — find session from 2026-02-21 (hackathon demo)
