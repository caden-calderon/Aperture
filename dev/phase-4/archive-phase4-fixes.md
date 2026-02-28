# Phase 4 Fix History — Archived

> All work in this file is COMPLETE. Archived from context.md to keep working docs lean.
> For current state, see `context.md`.

---

## All Completed Fixes (Cumulative)

**Rounds 1–4 (P0 blockers):**
- **CRITICAL-2**: `is_context_tool_name()` matches MCP-namespaced tool names
- **CRITICAL-1**: Turn-aware projection + partial-turn stubs in all 3 API formats
- **MEDIUM-1**: Model-aware session flip guard prevents Haiku from stealing active session
- **Cache root cause**: Manifest removed, threshold warnings only, batch-gated heuristics
- **Orphan tool_use sanitizer**: Prevents 400 errors from orphan tool_use after cleanup/archival
- **Session affinity**: Plan stage/commit/append carry session hints through MCP

**Rounds 5–6 (rewriter/cleanup pipeline):**
- **BUG #1**: `sanitize_anthropic_message_structure()` — merge consecutive roles, ensure user-at-end
- **BUG #3**: `serde_json` `preserve_order` feature — thinking block key order preserved
- **BUG #5**: Billing header filter before `content_fingerprint()` — stable system block IDs

**Round 8 (thinking block corruption — ALL VERIFIED R9):**
- **F2**: Guard thinking/redacted_thinking blocks in `replace_content_block_with_stub()` — early return
- **F3**: Exclude `Role::Thinking` from archival candidates + validation rejection
- **F1**: Pipeline reorder: stubs → replacements → removal (indices correct before shifts)
- **F4**: Skip merge for consecutive assistant messages with thinking blocks (insert synthetic user)
- **Fix B**: Filter context tool blocks from engine ingest (don't accumulate MCP tool blocks)
- **F6**: Tokenized multi-word search (split query into terms, score each independently)
- **F5**: Unknown plan parameter detection with helpful error listing expected params

**R9-1/R10 (plan layering):**
- `commit_staged_plan_for_session()` didn't update `persistent_archived_ids`
- Fix: Option B — `add_persistent_archives_for_session()` at commit time
  - `engine/planner/mod.rs:237-262`, `metacog/tools/plan.rs:248`
- R10 PASSED: 3 archive rounds stacked (8+8+5 = 21 blocks)

**R11–R14 (thread identity divergence — root cause + 3 fixes):**
- Root cause: After early-turn archival, POST-REWRITE body produces different `thread_identity` than PRE-REWRITE. MCP tools fall back to `active_session_id()` set from POST-REWRITE, but rewriter reads PRE-REWRITE session. Plans committed under session_B, rewriter reads session_A.
- **Fix 1**: Pass PRE-REWRITE thread_identity through capture exchange to ingest. `capture.rs:set_thread_identity()` + `handler.rs:471-481`
- **Fix 2**: Guard breadcrumb on `pending_plan.is_some()` — re-application shouldn't breadcrumb. `planner/mod.rs:547+651`
- **Fix 3**: Turn-aware MCP tool stripping — "last assistant message" boundary, strip stale MCP tools, preserve recent. `planner/cleanup.rs`
- R14 PASSED: 4+ cleans all fired correctly

**R16 (mutex safety pass):**
- 16 additional `.expect()` on Mutex locks fixed: RunawayGuard ×3, ActionLog ×7, CompressionQueue ×6 — all replaced with `.ok()` fallbacks

**R17 (WebKitGTK crash root cause):**
- Root cause: WebKitWebProcess crashes (SIGABRT in libgallium/Mesa) → tao calls `std::process::exit(0)` → all threads killed instantly
- Kill chain: WebKit crash → TaoWindowEvent::Destroyed → empty window list → RunEvent::ExitRequested → ControlFlow::Exit → process::exit(0)
- Quick fix: ExitRequested handler in lib.rs tracks user-initiated vs WebKit crash

**Proxy Decoupling — DONE (2026-02-24):**
- `aperture-proxy` binary owns engine + proxy (separate PID, survives all Tauri/WebKit crashes)
- Tauri spawns it as detached process (setsid on Unix), connects via HTTP/SSE
- `BroadcastDispatcher` → `tokio::sync::broadcast` → SSE at `/_aperture/events`
- `/_aperture/ipc/{command}` — HTTP IPC replaces all Tauri engine/hot-patch commands
- Frontend uses `invokeProxy()` (fetch) + `EventSource` instead of Tauri `invoke()`/`listen()`
- Terminal stays as Tauri IPC (PTY is process-local)
- 667 Rust tests + 53 frontend tests passing after decoupling

**File-Edit Crash — RESOLVED (2026-02-24):**
- Root cause: Tailwind v4 oxide scanner puts `.md` files into Vite module graph via `addWatchFile()`. Any markdown edit triggers `{ type: "full-reload" }` (HMR dead end) → WebKitGTK SIGABRT in Mesa/libgallium during GPU repaint
- Fix: Expanded `vite.config.js` `server.watch.ignored` to cover `**/*.md`, `**/dev/**`, `**/docs/**`, `**/.claude/**`, `**/.context/**`, `**/target/**`

---

## Manual Test Log

- **R14 (2026-02-23)** — 3–4 plan cycles worked. Pausing pattern observed after MCP tool calls.
- **MT1–MT5/R17 (2026-02-24)** — Crash root cause traced. File-edit mechanisms confirmed (WebKit crash + Vite HMR reload). Proxy decoupling implemented and verified.

## Diagnostic Reports

| Round | File | Key Finding |
|-------|------|-------------|
| 1–2 | `deep-dive-diagnostics-round-{1,2}-2026-02-19.md` | Projection mismatch, cleanup naming, session flips |
| 3 | `deep-dive-diagnostics-round-3-2026-02-19.md` | Elevated CRITICAL-1 |
| 4 | `deep-dive-diagnostics-round-4-consolidated-2026-02-19.md` | Cascading failure chain proven |
| 5 | `deep-dive-diagnostics-round-5-2026-02-19.md` | 3 new P0/P1 bugs in rewriter/cleanup pipeline |
| 10 | `deep-dive-diagnostics-round-10-2026-02-19.md` | R9-1 root cause confirmed, Option B fix |
