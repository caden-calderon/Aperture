# Phase 3 Remediation Plan (2026-02-13)

## Objective
Resolve staff-review findings so Phase 3 dynamic context shifting is behaviorally correct, durable between turns, and consistent across Claude MCP + Codex proxy runtimes.

## Decision Summary
- Prioritize correctness over feature expansion.
- Do not begin Phase 4 compression automation until Wave 1+2 are complete.
- Keep fail-open proxy behavior, but make state transitions deterministic and testable.

## Work Waves

### Wave 1 — Interception and Runtime Correctness
- Fix budget ceiling plumbing in planner heuristics.
- Fix re-invoke lifecycle ordering in `interceptor.rs`.
- Ensure captured/returned response parity after interception.
- Add integration tests that explicitly exercise re-invoke success/fallback paths.

Exit criteria:
- Re-invoke flows are deterministic and covered by tests.
- Budget ceiling setting changes planner behavior in tests.

### Wave 2 — Durable Between-Turn State
- Persist archive/compress/update mutations into engine state.
- Align request capture + rewrite + ingest ordering with durable mutation semantics.
- Wire real planner signals (current files, prior files, file mutations, task boundary).

Exit criteria:
- Archived/compressed/updated states persist across turns.
- File mutation tracking works in full runtime path, not just isolated tests.

### Wave 3 — Contract and UX Alignment
- Remove MCP schema drift by using shared tool definitions as source of truth.
- Align UI threshold labels with planner policy.
- Pass `budgetCeiling` to budget bar and validate rendering.
- Remove current warnings and harden weak tests.

Exit criteria:
- Shared schema parity across runtimes.
- `npm run check` warnings from touched files are resolved.

## Risk Notes
- Biggest risk is introducing regressions in hot proxy forwarding paths. Maintain strict fail-open behavior and keep proxy integration tests green after each step.
