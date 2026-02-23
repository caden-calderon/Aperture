# Round 12 — Regression Confirmed (2026-02-21)

## Context
Second manual test of the day. Same codebase as R11. All fixes in place.

## Observations
- 2 successful cleans (rounds 1 and 2 fired correctly)
- 3rd round had problems (same pattern as R11)

## Analysis
Confirms R11 was not a fluke. The 3rd-round failure is a consistent, reproducible regression. R10's success with 3 rounds (8+8+5) may have been a different code path (different block counts? different timing?) or the bug may be timing-sensitive and R10 was a lucky pass.

## Log Location
`~/.claude/projects/-home-caden-projects-Aperture/` — find session from 2026-02-21 (second test)
