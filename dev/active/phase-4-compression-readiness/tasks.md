# Phase 4 Compression Tasks (2026-02-14)

## Checkpoint A: Foundations
- [x] Define exact Rust config types for compression backend/model selection.
- [x] Add provider-aware default routing policy for sidekick model selection.
- [x] Add compression provider trait with fail-open helper semantics.
- [x] Add async compression queue contract types/state machine.
- [x] Add engine-owned compression settings getter/setter.
- [x] Define Tauri IPC for reading/updating compression settings.
- [x] Add UI placement and UX copy for compression sidekick settings.
- [x] Add backend-failure fail-open tests (provider layer) and settings normalization tests.

## Checkpoint B: Provider Execution (Next)
- [ ] Implement real Anthropic compression adapter.
- [ ] Implement real OpenAI compression adapter.
- [ ] Implement optional OpenRouter adapter with explicit config guardrails.
- [ ] Add queue worker execution loop with non-blocking scheduling.
- [ ] Add integration tests for provider timeout/error fail-open behavior.

## Checkpoint C: Planner Integration
- [ ] Convert eligible autonomous archival/compression actions into queue jobs (policy-gated).
- [ ] Enforce preserve-keys prompt contract before applying compressed variants.
- [ ] Add engine-side apply path for sidekick-produced summaries.
- [ ] Add regression tests to ensure no Phase 3 behavior loss.

## Checkpoint D: Quality + UX
- [ ] Add compression quality scoring and rejection thresholds.
- [ ] Add queue/sidekick status telemetry surfaces in UI.
- [ ] Add recommendation hooks for low-quality/failed compressions.

## Verification / Triage
- [ ] Reproduce and isolate orphan `tool_result` MCP smoke-test error on Anthropic path.
- [ ] Confirm Aperture context tool lifecycle still preserves valid tool_use/tool_result pairing after cleanup.
- [ ] Add/extend regression coverage for this failure mode once root cause is identified.

## Validation (Checkpoint A)
- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo test`
- [x] `npx vitest run`
- [x] `npm run check`
