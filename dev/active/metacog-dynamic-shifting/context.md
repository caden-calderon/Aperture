# Phase 3 Remediation Context (2026-02-14)

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
- Manual Phase 3 smoke-testing is in progress while Phase 4 starts.
- MCP smoke-test issue (Anthropic orphan `tool_result`) is now mitigated in code:
  - Autonomous heuristics no longer archive `tool_use` / `tool_result` blocks.
  - Rewriter now sanitizes orphan Anthropic `tool_result` blocks before forwarding.
  - Parser now generates deterministic block IDs (instead of random UUIDs) so IDs remain stable across identical parses and turn-appends.
  - `aperture_context_plan` now supports staged strategic flow (`stage/append/preview/commit/discard`) with heuristic suppression while staged.
  - Added operator recovery control (`engine_clear_context` + Settings “Clear Archive + Sessions”) to quickly reset corrupted test state between runs.
- Status: pending manual confirmation in live Claude session.
- New critical blocker discovered during live validation:
  - severe token/cost incident with high request fan-out and extreme cache token churn in Claude sessions.
  - this is currently the top-priority diagnostic item before continued Phase 4 expansion.

## Incident Addendum (2026-02-14)
- Aggregate evidence from recent local Claude session logs (deduped request IDs):
  - `requests=375`
  - `cache_creation_input_tokens=822,460`
  - `cache_read_input_tokens=46,096,657`
  - `total_including_cache=46,919,745`
- Focused validation session (`401b10df...`) showed:
  - `46` unique requests in ~10 minutes
  - `5,339,090` cache creation tokens
  - `0` cache read tokens
  - per-request cache creation growth into 100k+ range.
- Interpretation:
  - Major burn appears driven by model/tool request fan-out and large cached prompt prefix behavior.
  - not primarily explained by Aperture archive persistence.
- Scope note:
  - Largest burn sessions inspected had no `aperture_context_*` calls, indicating broader runtime/tool orchestration overhead outside direct Aperture context-tool usage.

## Incident Remediation Addendum (2026-02-14, P0)
- Exact trigger path identified in local logs:
  - focused session `401b10df...`: `46` unique requests in ~10 minutes (`~4.57 req/min`), `45/46` with tool calls.
  - heavy session `88e1b95d...`: `121` unique requests in ~14.6 minutes (`~8.30 req/min`), `121/121` with tool calls.
  - queue/progress amplification in `d27ec2d0...`: `412` queue-operation lines with only `3` unique payloads; large repeated task notifications.
- Aperture-side mitigations implemented:
  - low-overhead context tool argument validation (`read/search/plan`) before dispatch.
  - output-size controls/truncation for context tool responses (`preview/read/search/status`) with compact mode under burst conditions.
  - proxy runaway-guard detector for sustained request bursts (warning-only, fail-open preserved) plus compact fallback for high-cost context-tool calls during hard bursts.
- Architecture boundary clarified:
  - Claude/provider request fan-out behavior and queue notification duplication remain external.
  - Aperture now limits additional local prompt bloat and surfaces operator-visible warnings during runaway patterns.

## Project Direction Update (2026-02-15)
- Phase 4 execution is now gated by token-economics parity.
- Expansion work (new providers/queueed autonomous compression) is paused until Aperture is measured at or below baseline Claude Code token usage on benchmark tasks.
- Next-phase architecture emphasis is delta-based context transport plus session-level ROI control, not additional guardrail layering alone.

## Source Artifacts
- Staff review: `dev/active/metacog-dynamic-shifting/staff-review-2026-02-13.md`
- Design: `dev/active/metacog-dynamic-shifting/design.md`
- Phase spec: `.context/phases/phase-3.md`
