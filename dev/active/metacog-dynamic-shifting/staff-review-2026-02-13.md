# Phase 3 Staff Review Report (2026-02-13)

## Scope
Review target:
- Uncommitted Phase 3 implementation from Checkpoints E/F/G (planner, runtimes, rewriter, interceptor, MCP bridge, UI settings).
- Architecture fit against `dev/active/metacog-dynamic-shifting/design.md` and `.context/phases/phase-3.md`.
- Code quality, correctness, lifecycle behavior, and test sufficiency.

Reviewed areas:
- `src-tauri/src/engine/planner/*`
- `src-tauri/src/metacog/*`
- `src-tauri/src/proxy/{handler,rewriter,interceptor,context_api,parser}.rs`
- `src-tauri/src/bin/aperture_mcp.rs`
- `src/lib/stores/context.svelte.ts`
- `src/lib/components/{settings/SettingsPanel.svelte,ui/TokenBudgetBar.svelte,layout/TitleBar.svelte,blocks/ContextBlock.svelte}`
- `src-tauri/tests/tool_lifecycle_integration.rs`

## Verification Run
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check` ✅
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` ✅
- `cargo test --manifest-path src-tauri/Cargo.toml --test tool_lifecycle_integration` ✅ (9/9)
- `cargo test --manifest-path src-tauri/Cargo.toml --test proxy_flow` ✅ (17/17; run with elevated permissions)
- `npx vitest run` ✅ (44/44)
- `npm run check` ✅ (0 errors, 2 warnings)

## Findings (Ordered by Severity)

### Critical
1. Budget ceiling does not affect planner thresholds at runtime.
- `ContextPlanner::set_budget_ceiling()` stores an override, but `plan()` still uses `self.config` directly for heuristics.
- Result: UI setting appears saved but behavior remains default threshold policy.
- Refs: `src-tauri/src/engine/planner/mod.rs:77`, `src-tauri/src/engine/planner/mod.rs:143`, `src/lib/stores/context.svelte.ts:846`

2. Re-invoke loop currently removes context call/result context before upstream re-invoke.
- `reinvoke_with_results()` appends assistant tool calls + tool results, then calls `cleanup_history()` prior to re-send.
- This can strip the very tool lifecycle data required for a correct follow-up completion.
- Refs: `src-tauri/src/proxy/interceptor.rs:300`, `src-tauri/src/proxy/interceptor.rs:303`, `src-tauri/src/proxy/interceptor.rs:307`

3. Context mutations are not durably represented as engine state across turns.
- Archive/compress/update are converted to payload rewrite decisions but not persisted as engine block state; only zone/pin have internal engine updates.
- Capture of request body occurs before rewriting, so engine ingest path can continue carrying pre-rewrite semantics.
- This conflicts with the Phase 3 contract that changes apply between turns.
- Refs: `src-tauri/src/engine/planner/applicator.rs:64`, `src-tauri/src/engine/planner/applicator.rs:72`, `src-tauri/src/engine/mod.rs:760`, `src-tauri/src/proxy/handler.rs:598`, `src-tauri/src/proxy/handler.rs:611`

### High
4. Capture/finalization after interception records original upstream response, not modified response body.
- When interception succeeds, handler captures `response_bytes` and finalizes exchange before returning modified body.
- UI/debug replay can diverge from what client actually consumed.
- Ref: `src-tauri/src/proxy/handler.rs:450`

5. Planner signals and file mutation wiring are incomplete in real runtime path.
- `PlannerInput` from rewriter uses default signals and `file_mutations: None`; task boundary and file-diff heuristics are not sourced from actual proxy traffic.
- Ref: `src-tauri/src/proxy/rewriter.rs:52`, `src-tauri/src/proxy/rewriter.rs:57`

### Medium
6. MCP tool schema drifts from shared tool schema (`split` missing in MCP plan schema).
- Shared runtime schema includes `split`, MCP binary schema does not.
- Creates capability inconsistency across Claude vs Codex paths.
- Refs: `src-tauri/src/metacog/runtime.rs:233`, `src-tauri/src/bin/aperture_mcp.rs:90`

7. Frontend threshold display math does not match backend policy and budget ceiling is not passed into budget bar usage.
- Settings/UI uses 75/88/95 multipliers while planner policy is 50/80/100 of ceiling.
- `TokenBudgetBar` accepts `budgetCeiling`, but call site does not pass it.
- Refs: `src-tauri/src/engine/planner/types.rs:243`, `src/lib/components/settings/SettingsPanel.svelte:14`, `src/lib/components/ui/TokenBudgetBar.svelte:74`, `src/routes/+page.svelte:209`

### Low
8. `ContextBlock` introduces `.block.archived` styles with no active class toggling path.
- This is currently dead CSS and contributes to warning noise.
- Ref: `src/lib/components/blocks/ContextBlock.svelte:328`

9. Accessibility warning on icon-only close button in settings panel.
- Missing label/title for close button.
- Ref: `src/lib/components/settings/SettingsPanel.svelte:40`

10. Integration tests include weak assertions that can mask regressions.
- Some tests only assert conditionally if rewritten body exists.
- One assertion is tautological.
- Refs: `src-tauri/tests/tool_lifecycle_integration.rs:228`, `src-tauri/tests/tool_lifecycle_integration.rs:276`, `src-tauri/tests/tool_lifecycle_integration.rs:512`

## Architecture Assessment
Strengths:
- Clear module boundaries: planner/runtime/proxy split is understandable and extensible.
- Good baseline test volume and lint discipline.
- Runtime abstraction is coherent across Claude/Codex/Passive surfaces.

Primary architectural gap:
- State coherence across request rewrite, interception/reinvoke, capture, and engine persistence is not fully closed. This is the main blocker for Phase 3’s “continuous dynamic shifting” promise.

## Remediation Plan

### Wave 1 (Stabilize Correctness)
1. Fix budget ceiling plumbing in planner heuristics.
2. Correct re-invoke flow to preserve required assistant/tool lifecycle context until loop terminates.
3. Capture the effective response body after interception/reinvoke path.
4. Add focused tests for re-invoke depth/timeouts and mixed-tool behavior.

Acceptance criteria:
- Budget ceiling changes produce measurable threshold changes in planner output.
- Context-only re-invoke path completes without premature cleanup.
- Captured response equals returned response for intercepted calls.

### Wave 2 (State Coherence)
1. Persist archive/compress/update semantics into engine-side state between turns.
2. Ensure engine ingest/capture order aligns with rewritten payload semantics.
3. Wire file mutation + task-boundary signals from proxy traffic into `PlannerInput`.

Acceptance criteria:
- Archived/compressed blocks remain changed on the next request without re-derivation.
- File edits propagate to block content in real flow, not only planner unit tests.

### Wave 3 (Contract Alignment + Hardening)
1. Unify MCP tool schema generation with shared runtime tool definitions.
2. Align frontend threshold visuals with backend config policy.
3. Strengthen integration tests to require rewrite expectations and remove tautologies.
4. Resolve remaining UI warnings.

Acceptance criteria:
- Claude/Codex tool contracts are schema-equivalent.
- `npm run check` has no avoidable warnings from touched components.

## Recommended Execution Order
1. Wave 1 (correctness and interception)
2. Wave 2 (durable state and planner signals)
3. Wave 3 (contract alignment and polish)

## References
- Design: `dev/active/metacog-dynamic-shifting/design.md`
- Phase spec: `.context/phases/phase-3.md`
- Detailed test scenarios: `dev/active/metacog-dynamic-shifting/test-playbook.md`
