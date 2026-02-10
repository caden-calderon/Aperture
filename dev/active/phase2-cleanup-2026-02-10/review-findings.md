# Phase 2 Code Review Findings

> Staff-level review of all Phase 2 modified files, conducted 2026-02-10.
> Baseline: 248 tests passing, clippy clean, svelte-check clean.

---

## 1. Proxy Layer Review

### handler.rs (~820 lines) — NEEDS SPLITTING

**Issue 1.1: `forward_request` is 373 lines (HIGH)**
- Lines 179-551, handles 8 concerns: body reading, zstd decompression, hot-patch, capture, header forwarding, timing, streaming vs non-streaming paths, SSE tee logic
- Suggested split: `prepare_request_body()`, `handle_streaming_response()`, `handle_non_streaming_response()`, keep `forward_request()` as orchestrator

**Issue 1.2: SSE finalization silent fail (MEDIUM)**
- Lines 440-474: If `finalize_streaming` returns None, streaming response is silently dropped — no event emitted, frontend never knows stream completed
- Fix: Emit event or log warning if no exchange found

**Issue 1.3: `determine_upstream` routing duplication (MEDIUM)**
- Path detection split across handler.rs, parser.rs, capture.rs, provider_adapter.rs
- Not a bug, but document that `determine_upstream` is authoritative

**Issue 1.4: Log message asymmetry (LOW)**
- Lines 34, 54: `-->` vs `<--` format slightly inconsistent

### capture.rs (~720 lines) — GOOD

**Issue 2.1: `evict_if_needed` has redundant break (MEDIUM)**
- Lines 225-251: `take(to_remove)` already limits, but loop has redundant `break`
- DashMap iteration order is not insertion-ordered, so "FIFO eviction" claim is inaccurate

**Issue 2.2: Model loss in OpenAI streams (MEDIUM)**
- Lines 366-430: Model only set from first event containing it. If first event lacks model, final response has null model
- Fix: Remove `is_none()` guard on model assignment

**Issue 2.3: `extract_final_response` API confusion (LOW)**
- Lines 266-277: Anthropic path doesn't use `path` param but OpenAI does

**Issue 2.4: SSE JSON parse errors silent (LOW)**
- No debug log on malformed SSE events

### hot_patch.rs (~350 lines) — GOOD

**Issue 3.1: `drain()` is dead code (LOW)**
- Lines 58-64: Replaced by `peek_all()` in Phase 2, never called

**Issue 3.2: Error message misleading (LOW)**
- Lines 99-114: "is the body compressed?" when caller already handles zstd

**Issue 3.3: Inefficient role cloning (LOW)**
- Lines 156-160: Allocates String per message for comparison, should use `&str`

### mod.rs (~202 lines) — GOOD

**Issue 4.1: `new()` duplicates `with_config()` (LOW)**
- Lines 94-118: `new()` should just call `with_config(Default::default())`

**Issue 4.2: Builder pattern incomplete (LOW)**
- Lines 134-142: `hot_patches` and `engine` set via direct mutation, not builder methods

**Issue 4.3: Test missing chatgpt_codex_url assertion (LOW)**
- Lines 174-184

### provider_adapter.rs (~252 lines) — GOOD

**Issue 5.1: Adapter list rebuilt per call (LOW)**
- Lines 201-208: `builtin_parser_adapters()` allocates 5 Box objects per call
- Not used in Phase 2 hot path, Phase 3 concern

**Issue 5.2: Placeholder capabilities are guesses (LOW)**
- Lines 171-197: GeminiCli, OpenCode, KiloCode capabilities are assumptions

### Cross-cutting

**Pattern: Event dispatch without validation (MEDIUM)**
- handler.rs lines 62, 290, 398, 441, 450, 509, 518
- Events dispatched without checking if actually sent; broken channel = silent frontend miss

---

## 2. Engine Review

### mod.rs (1,326 lines) — GOD MODULE

**Issue 6.1: God module needs splitting (HIGH)**
- Consolidates: block mutation API, bulk operations, session management, session identity tracking, undo, persistence coordination, event emission, ingestion with pipeline orchestration
- Recommended extraction:
  - `mutations.rs` — single/bulk block mutations (~300 lines)
  - `sessions.rs` — session lifecycle (~100 lines)
  - `persistence.rs` — persistence coordination (~80 lines)
  - Remaining `mod.rs` — core orchestration (~250 lines)

**Issue 6.2: Silent persistence failures (HIGH)**
- DB init and persistence failures logged but silently continue
- UI thinks blocks saved, backend never persisted

**Issue 6.3: No-op mutation pattern (MEDIUM)**
- Early returns for no-ops don't log action records — intentional but undocumented

### versioning.rs (259 lines)

**Issue 7.1: Broken ISO 8601 timestamp (HIGH)**
- `iso_now()` returns Unix seconds (e.g., "1736000000"), not ISO 8601
- Three different timestamp formats across engine modules

### storage.rs (587 lines)

**Issue 8.1: Incomplete session loading (MEDIUM)**
- `source` hardcoded to "unknown", `thread_identity` to None on load
- Data loss on app restart: session identity lost

**Issue 8.2: Dangerous parse_edit_source default (MEDIUM)**
- Unknown variants silently default to `EditSource::User`

**Issue 8.3: Duplicate save_block/save_blocks code (LOW)**

### session.rs (385 lines)

**Issue 9.1: Lock poisoning panic (MEDIUM)**
- Five `.expect("active_id lock poisoned")` calls

### policy.rs (296 lines)

**Issue 10.1: `ClearSession` action defined but never called (MEDIUM)**
- Orphaned code

### Other engine modules (block.rs, types.rs, store.rs, budget.rs, staleness.rs, zone.rs, dependency.rs, action_log.rs, tokens.rs, pipeline.rs)
- All excellent. No critical issues. Well-tested.

---

## 3. Frontend Review

### context.svelte.ts — 7 issues

**Issue 11.1: Infinite loop risk in snapshot switching (HIGH)**
- `switchToSnapshot`/`switchToWorkingState` stores snapshots in `workingStateCache` ($state) and restores to `blocks` ($state)
- Classic Svelte 5 infinite loop if any $effect reads both

**Issue 11.2: Silent error swallowing on invoke() calls (MEDIUM)**
- Engine invocations silently catch and ignore errors

**Issue 11.3: Other issues (MEDIUM/LOW)**
- Single-callback registration pattern (race condition)
- `hasCapturedTraffic` not session-scoped

### +page.svelte — 7 issues

**Issue 12.1: $effect token recalculation without dependency guard (HIGH)**
- Lines 168-177: Recalculates token snapshots on every block change without checking if totals actually changed

**Issue 12.2: `switchingSession` flag deadlock risk (HIGH)**
- No timeout protection if Tauri invoke hangs

**Issue 12.3: `Promise.all` in refreshEngineState (MEDIUM)**
- All-or-nothing failures

### connection.svelte.ts — 5 issues (MEDIUM/LOW)
### selection.svelte.ts — 3 issues (MEDIUM/LOW)
### terminal.svelte.ts — 3 issues (MEDIUM/LOW)
### providerAdapters.ts — 2 issues (LOW)
### blockConvert.ts — 3 issues (MEDIUM/LOW)

---

## 4. Ancillary Files Review

### codex_bridge.rs (932 lines)

**Issue 13.1: digest_blocks() non-deterministic hash (HIGH/BUG)**
- Uses `DefaultHasher` which is randomly seeded per process
- Same blocks hash differently across runs → false-positive "changed" on every poll
- Fix: Use deterministic hasher or direct comparison

**Issue 13.2: Hand-rolled now_iso8601() date formatter (MEDIUM)**
- Lines 702-727: Complex modular arithmetic, no references
- Replace with standard library call

**Issue 13.3: estimate_tokens() inconsistent with engine (MEDIUM)**
- Uses `content.len() / 4.0` vs engine's actual tokenizer

**Issue 13.4: Duplicate history file parsing (MEDIUM)**
- Two functions parse same file format, not DRY

### terminal/mod.rs

**Issue 14.1: PROXY_PORT duplicated (MEDIUM)**
- `PROXY_PORT: u16 = 5400` duplicates `proxy::DEFAULT_PORT`
- Can silently diverge if one is changed

**Issue 14.2: Unknown provider silently returns empty EnvPlan (MEDIUM)**

### lib.rs

**Issue 15.1: parse_zone() no validation (MEDIUM)**
- Any string becomes a custom zone, no validation

**Issue 15.2: Tauri command error handling lossy (MEDIUM)**
- `.unwrap_or_default()` on serialization silently swallows errors

**Issue 15.3: Proxy startup race condition (MEDIUM)**
- Logs "listening" before proxy thread confirms startup

### events/dispatcher.rs

**Issue 16.1: Duplicate emit helpers (MEDIUM)**
- EventDispatcher and DynDispatcher have ~47 lines of identical methods

### events/types.rs

**Issue 17.1: STREAM_PROGRESS unused (LOW)**

### Tests

**Issue 18.1: Latency test hardcodes 25ms threshold (MEDIUM)**
- Flaky on slower machines/CI

**Issue 18.2: Engine session tests limited (MEDIUM)**
- Only 2 scenarios, no multi-provider/model coverage

---

## Priority Summary

### Must Fix (Correctness)
1. digest_blocks() non-deterministic hash (BUG)
2. versioning.rs iso_now() broken timestamp (BUG)
3. PROXY_PORT duplication (bug risk)

### Should Fix (Cleanup Scope)
4. handler.rs forward_request() split (373 → ~4 functions)
5. Dead code removal (drain(), ClearSession, STREAM_PROGRESS)
6. DRY violations (new()/with_config(), role cloning, eviction break)
7. Consolidate timestamp utilities

### Defer to Phase 3
- engine/mod.rs god module split (large refactor, tests all passing)
- storage.rs session loading improvements (needs schema migration)
- Frontend infinite loop risk (needs careful Svelte 5 analysis)
- Error handling improvements across all IPC commands
- Dispatcher trait extraction
