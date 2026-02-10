# Provider Modularity Tasks - 2026-02-09

## Next Session Checklist

## Architecture
- [x] Define or confirm a single provider adapter contract (launch, lifecycle, parse, capabilities, health mapping).
- [x] Verify core engine/store remains provider-neutral.
- [x] Move any leaked provider conditionals to adapter/parser boundaries.

## Claude + Codex Parity
- [x] Verify manual terminal launch and button launch follow the same bridge path.
- [x] Verify disconnect/provider-switch clears context and status consistently.
- [x] Verify launch status states are deterministic and sourced from one reducer.

## Expansion Readiness
- [x] Add provider onboarding template/checklist for Gemini CLI, OpenCode, KiloCode.
- [x] Ensure adding a provider requires no core store edits.
- [x] Add adapter capability matrix doc (available metadata, reasoning visibility, usage counters).

## Performance + Reliability
- [x] Re-check proxy and stream hot paths for blocking calls.
- [x] Keep parsing/enrichment off forwarding critical path when possible.
- [x] Confirm instrumentation exists for request/stream timing regressions.

## Validation
- [x] Run `make check`
- [x] Run `npm run build`
- [x] Add/adjust tests for provider lifecycle transitions and parser normalization edge cases.

## Follow-up Tasks
- [x] Add backend parser adapter trait to mirror frontend provider adapter contract for Gemini CLI/OpenCode/KiloCode.
- [x] Add explicit terminal/provider lifecycle tests around manual `claude` command detection and status convergence.
- [x] Add capability flags (`supports_usage`, `supports_reasoning`, `supports_resume_id`) to the provider adapter contract and expose in UI.

## Phase 2 Cleanup Pass (2026-02-09)

### Completed
- [x] Fix undo regression in version store: undo now restores the latest pre-edit snapshot (single-edit undo works).
- [x] Fix ingest replacement hygiene: clear stale dependency edges and version history when replacing session blocks.
- [x] Tighten session matching: provider + model now determines session reuse (prevents cross-model session collisions).
- [x] Fix SQLite replacement behavior: `save_blocks(..., Some(session_id))` now replaces prior session rows instead of accumulating stale rows.
- [x] Harden tokenizer path: removed panic-on-init in token counting; tokenizer init failures now fall back to heuristic counting.
- [x] Make frontend engine-backed mutations policy-aware (`move/remove/pin/compress/edit` only apply locally when engine allows).
- [x] Emit `context_updated` on pin changes for consistent frontend refresh behavior.
- [x] Add regression tests:
  - Rust: ingest cleanup, single-edit undo, provider+model session split, SQLite session-row replacement.
  - Frontend: policy-gated remove/move behavior.
- [x] Keep session metadata accurate after UI mutations: session token totals/block IDs now refresh after edit/remove/undo and persist consistently.
- [x] Wire policy-confirmation mutation flow end-to-end: backend `confirmed` path + frontend confirm/retry handling.
- [x] Add engine bulk mutation IPC (`bulk_remove`, `bulk_move`) and use it for multi-select operations.
- [x] Remove false-success UX paths: UI now shows success only when mutations are actually applied, and warns on policy blocks.
- [x] Add confirmation/bulk regression tests:
  - Rust: pinned-remove confirmation gate, bulk-remove confirmation gate, session metadata sync after post-ingest mutations.
  - Frontend: confirmation cancel/retry behavior and bulk IPC path coverage.

### Follow-up
- [ ] Consider background/off-thread persistence for non-streaming ingest path to minimize tail latency under heavy SQLite IO.

## Phase 2 Re-Audit (2026-02-09, Session 2)

### Completed
- [x] Re-audited Phase 2 against `.context/phases/phase-2.md` success criteria across engine IPC + frontend integration paths.
- [x] Fixed no-op mutation behavior in engine (`update_content`, `move_block`, `pin_block`, `compress_block`, `bulk_move`) so idempotent requests no longer emit policy/action/version side effects.
- [x] Fixed frontend no-op mutation behavior (`moveBlock`, `moveBlocks`, `updateBlockContent`, `setCompressionLevel`, `pinBlock`) so no-op actions skip engine IPC and local edit-history churn.
- [x] Fixed engine-sync edge case: `setEngineBlocks([])` now clears working-state blocks (stale UI state removed).
- [x] Added frontend regression coverage for no-op mutation IPC skipping and empty engine-sync clearing (`src/lib/stores/context-policy.test.ts`).
- [x] Added Rust regression coverage for no-op mutation side-effect suppression (`src-tauri/src/engine/mod.rs` tests).
- [x] Added UI session switching support (fetch/list/switch active engine sessions in `src/routes/+page.svelte`) to satisfy Phase 2 session-switching criterion.
- [x] Added token-accuracy threshold coverage (`<=2%` vs reference tokenizers across model families) in `src-tauri/src/engine/tokens.rs`.
- [x] Optimized pipeline heuristic application to O(n) update path and added runtime guard test ensuring average classify runtime stays `<2ms` for typical batch (`src-tauri/src/engine/pipeline.rs`).
- [x] Added staleness-score visibility in UI (store-derived staleness formula + per-block displayed score in `ContextBlock`).
- [x] Full validation green: `make check`, `npm run build`.

### Remaining Risk
- [ ] Manual Phase 2 acceptance items still require explicit sign-off evidence (provider API token-count spot-check and manual UI verification checklist execution).
- [ ] Background/off-thread persistence for heavy SQLite IO remains open and should stay tracked before/into Phase 3.

## Phase 2 Re-Audit (2026-02-09, Session 3)

### Completed
- [x] Fixed live block-edit persistence regression by making local hot-patch overlays session-scoped in `src/lib/stores/context.svelte.ts` (prevents cross-session content rewrite leakage).
- [x] Added frontend regression coverage for session-scoped overlay behavior in `src/lib/stores/context-live.test.ts`.
- [x] Fixed stale-selection behavior after engine block-ID replacement by adding explicit selection pruning hooks and wiring refresh pruning in `src/routes/+page.svelte`.
- [x] Added selection pruning regression in `src/lib/stores/context-selection.test.ts`.
- [x] Optimized engine refresh path in `src/routes/+page.svelte` with parallel IPC fetch + active-session-first state apply.
- [x] Validation green after fixes: `make check`, `npm run build`.

### Remaining Risk
- [ ] Manual Phase 2 acceptance items still require explicit sign-off evidence (provider API token-count spot-check and manual UI verification checklist execution).
- [ ] Background/off-thread persistence for heavy SQLite IO remains open and should stay tracked before/into Phase 3.

## Phase 2 Re-Audit (2026-02-09, Session 5)

### Completed
- [x] Added direct-mode context edit guardrails in `src/lib/stores/context.svelte.ts` so direct/observational mode blocks engine-backed mutations before local state changes or hot-patch queuing.
- [x] Added explicit mode badge/state in `src/routes/+page.svelte` and transport-derived mode mapping in `src/lib/stores/terminal.svelte.ts` + `src/lib/utils/providerAdapters.ts` (`Direct (Read-Only)` vs `Proxy (Mutable)`).
- [x] Added UI-side read-only guards for synchronous edits to avoid false-success UX (`src/lib/composables/blockHandlers.svelte.ts`, `src/lib/composables/modalHandlers.svelte.ts`, `src/lib/components/controls/BlockTypeManager.svelte`).
- [x] Isolated engine session identity by source/thread in `src-tauri/src/engine/mod.rs`; direct Codex bridge ingest now tags `source=direct_cli_bridge` and thread id in `src-tauri/src/terminal/codex_bridge.rs`.
- [x] Updated proxy ingest wiring to explicit source tagging (`source=proxy`) in `src-tauri/src/proxy/handler.rs`.
- [x] Added/updated regression coverage:
  - Frontend: direct-mode guardrail + mode mapping/state (`src/lib/stores/context-policy.test.ts`, `src/lib/stores/terminal-mode.test.ts`, `src/lib/utils/providerAdapters.test.ts`).
  - Rust integration: multi-thread session isolation (`src-tauri/tests/engine_session_isolation.rs`).
  - Rust unit: source/thread identity matching semantics (`src-tauri/src/engine/mod.rs` tests).
- [x] Validation green after fixes: `make check`, `npm run build`.

### Remaining Risk
- [ ] Manual Phase 2 acceptance items still require explicit sign-off evidence (provider API token-count spot-check and manual UI verification checklist execution).
- [ ] Background/off-thread persistence for heavy SQLite IO remains open and should stay tracked before/into Phase 3.

## Phase 2 Re-Audit (2026-02-09, Session 6)

### Completed
- [x] Upgraded Codex Direct from observational-only to bridge-mutable for content edits:
  - Added backend command `codex_direct_apply_content_edit` (`src-tauri/src/terminal/mod.rs`, `src-tauri/src/lib.rs`).
  - Added Codex app-server `sendUserMessage` mutation path + active conversation discovery from history (`src-tauri/src/terminal/codex_bridge.rs`).
- [x] Updated frontend mutation mode mapping so `openai` direct launch reports `Direct (Mutable)` (`src/lib/utils/providerAdapters.ts`, `src/lib/stores/terminal.svelte.ts`, `src/routes/+page.svelte`).
- [x] Updated context edit flow so direct mutable mode writes upstream via bridge (instead of queueing proxy hot patches) while preserving engine policy checks (`src/lib/stores/context.svelte.ts`).
- [x] Expanded regression coverage:
  - Frontend: direct mutable mode mapping and command routing (`src/lib/utils/providerAdapters.test.ts`, `src/lib/stores/terminal-mode.test.ts`, `src/lib/stores/context-policy.test.ts`).
  - Rust: Codex direct mutation helper behavior (`src-tauri/src/terminal/codex_bridge.rs` tests).
- [x] Validation green after fixes: `make check`, `npm run build`.

### Remaining Risk
- [ ] Direct-mutable parity is currently implemented for Codex Direct content edits; equivalent bridge-mutation paths for other non-proxy subscription clients (Gemini/OpenCode/KiloCode direct modes) still need provider-specific adapters.
- [ ] Manual Phase 2 acceptance items still require explicit sign-off evidence (provider API token-count spot-check and manual UI verification checklist execution).
- [ ] Background/off-thread persistence for heavy SQLite IO remains open and should stay tracked before/into Phase 3.

## Phase 2 Re-Audit (2026-02-09, Session 7)

### Completed
- [x] Fixed Codex Direct edit failures caused by unresolved conversation context:
  - Codex bridge now resolves conversations via `listConversations`, validates target conversation identity, resumes it explicitly, then sends mutation (`src-tauri/src/terminal/codex_bridge.rs`).
  - Direct edit command now accepts explicit conversation identity and surfaces a direct-edit-specific error type (`src-tauri/src/terminal/mod.rs`, `src-tauri/src/terminal/error.rs`).
- [x] Wired direct edits to the active engine thread identity to avoid cross-thread mutation targeting (`src/lib/stores/context.svelte.ts`, `src/routes/+page.svelte`).
- [x] Extended engine session metadata with `source` and `thread_identity` so frontend can target the exact direct thread (`src-tauri/src/engine/session.rs`, `src-tauri/src/engine/mod.rs`).
- [x] Fixed dev-launch Tailwind parse failure in context diff entry by replacing dynamic class-string interpolation with explicit class directives (`src/lib/components/features/ContextDiffEntry.svelte`).
- [x] Expanded regression coverage:
  - Frontend: direct bridge invocation now includes active conversation/thread identity (`src/lib/stores/context-policy.test.ts`).
  - Rust: conversation-id resolution tests in Codex bridge + session info exposure test in engine (`src-tauri/src/terminal/codex_bridge.rs`, `src-tauri/src/engine/mod.rs`).
- [x] Validation green after fixes: `make check`, `npm run build`.

### Remaining Risk
- [ ] Codex app-server `resumeConversation` can fail in environments with broken permissions under `~/.codex/sessions`; user environment hygiene still matters for direct mutability reliability.
- [ ] Direct-mutable parity is currently implemented for Codex Direct content edits; equivalent bridge-mutation paths for other non-proxy subscription clients (Gemini/OpenCode/KiloCode direct modes) still need provider-specific adapters.
- [ ] Manual Phase 2 acceptance items still require explicit sign-off evidence (provider API token-count spot-check and manual UI verification checklist execution).
- [ ] Background/off-thread persistence for heavy SQLite IO remains open and should stay tracked before/into Phase 3.
