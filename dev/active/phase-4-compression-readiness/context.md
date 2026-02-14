# Phase 4 Compression Readiness Context (2026-02-14)

## Current State
- Phase 3 metacognition/dynamic shifting is complete through Wave 3 remediation.
- Phase 4 has started with **Checkpoint A foundations implemented**.
- Manual validation pass is now running with Phase 3 smoke tests first, then Phase 4 progression.
- Proxy fail-open behavior is preserved; no request/response critical-path blocking was introduced.
- Existing planner/archive semantics remain unchanged in this checkpoint.

## Active Validation Note
- MCP/tool smoke testing surfaced a conversation-shape failure before Aperture tools could be fully exercised:
  - `invalid_request_error` with orphan `tool_result` / missing prior `tool_use`.
- This is currently treated as a Phase 3 verification blocker to clear before continuing deeper Phase 4 behavior testing.

## What Checkpoint A Added
- New `engine::compression` module with:
  - Typed compression backend/settings contract.
  - Provider routing defaults by active upstream provider.
  - Fail-open provider trait helpers.
  - Async queue contract (in-memory queue model, status lifecycle).
- `ContextEngine` now owns sidekick compression settings via thread-safe getter/setter.
- New Tauri IPC commands for compression settings read/update.
- Frontend store + settings panel now expose compression sidekick controls (backend/model/timeout/max tokens).
- Test coverage expanded for Rust compression foundations and frontend store behavior.

## Boundaries Enforced
- No regressions to Phase 3 planner lifecycle and context-tool interception paths.
- No automatic sidekick compression execution in this checkpoint yet.
- No change to existing archival-first heuristic behavior yet.

## Remaining Work (Phase 4)
- Wire real provider adapters (Anthropic/OpenAI/OpenRouter) to network-backed sidekick compression.
- Connect queue worker lifecycle to planner-driven autonomous compression actions.
- Implement preserve-keys prompting + summary quality scoring.
- Add telemetry and UI visibility for queue/failure outcomes.
