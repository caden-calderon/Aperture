# Phase 3 Remediation Context (2026-02-13)

## Current State
- Checkpoints A-G are implemented.
- Wave 1, Wave 2, and Wave 3 remediation are complete.
- Wave 1 completed items:
  - Planner now applies the runtime `budget_ceiling` override during heuristic threshold evaluation (`ContextPlanner::plan` uses effective config).
  - Re-invoke lifecycle ordering now preserves assistant context tool calls + injected tool results until loop completion (removed premature `cleanup_history()` in reinvoke path).
  - Non-streaming intercepted responses now capture/finalize with the effective returned body, not the original upstream body.
  - Added/strengthened tests for:
    - context-only re-invoke success path
    - mixed context + real tool stripping path
    - re-invoke depth-limit fail-open
    - re-invoke timeout fail-open
    - runtime budget ceiling override behavior in planner heuristics
- Wave 2 completed items:
  - Persisted archive/compress/update/expand semantics as durable engine-side mutations (`EngineUpdateKind` now applies archive/compression/content restoration/content updates to block state).
  - Reordered proxy request flow so capture occurs after rewrite, ensuring ingest receives effective forwarded semantics rather than pre-rewrite payloads.
  - Wired planner signals from real request traffic:
    - current-turn file signals sourced from parsed tool-call traffic
    - previous-turn file memory + task-boundary detection tracked in planner state
    - file mutation detection (`edit/write/delete`) passed through `PlannerInput.file_mutations`
  - Added round-trip persistence tests:
    - multi-turn durable archive/compress/update persistence in `tool_lifecycle_integration`
    - capture-after-rewrite semantics validation in `proxy_flow`
- Wave 3 completed items:
  - Removed MCP schema drift by generating `aperture-mcp` `tools/list` schemas from shared `context_tool_definitions()` source-of-truth.
  - Added MCP schema parity coverage for `aperture_context_plan.split`.
  - Aligned frontend threshold math to planner policy (soft/medium/hard = 50%/80%/100% of configured budget ceiling).
  - Passed `budgetCeiling` through to `TokenBudgetBar` usage site (`src/routes/+page.svelte`) so marker rendering reflects runtime ceiling.
  - Replaced weak optional assertions in `tool_lifecycle_integration` with strict rewrite/tool-array expectations.
  - Resolved current Svelte warnings in touched components:
    - removed unused `.block.archived` selector in `ContextBlock.svelte`
    - added explicit accessible label/title to settings close button in `SettingsPanel.svelte`

## Why This Matters
Phase 3’s value proposition is continuous, reliable context optimization. If mutations are not durably applied between turns or re-invoke behavior is inconsistent, model trust in the system degrades and phase goals are only partially met.

## Validation Snapshot
- `cargo fmt --check` ✅
- `cargo clippy -- -D warnings` ✅
- `cargo test` ✅ (452 lib + 6 bin + 2 session + 21 proxy_flow + 10 tool_lifecycle + 0 doc)
- `npx vitest run` ✅ (47/47)
- `npm run check` ✅ 0 errors, 0 warnings

## Remaining Blockers
- No Wave 3 blockers remain.
- Phase 4 work has not started (intentionally out of scope for this remediation wave).

## Source Artifacts
- Staff review: `dev/active/metacog-dynamic-shifting/staff-review-2026-02-13.md`
- Design: `dev/active/metacog-dynamic-shifting/design.md`
- Phase spec: `.context/phases/phase-3.md`
