# Phase 4 Token Economics Context

## Current State (2026-02-24)

**PROXY DECOUPLING COMPLETE but file-edit crash PERSISTS.** Next session: deep dive diagnostic.
**ALL 3 FIXES VERIFIED IN R14 MANUAL TEST. Plan layering regression RESOLVED.**

After 3 sessions of deep diagnostic work (R11→R13), the root cause is proven by integration test:

**Bug**: After cumulative archival removes early turns, POST-REWRITE body produces a different `thread_identity` than PRE-REWRITE body. MCP tools fall back to `active_session_id()` (set by ingest from POST-REWRITE), while the rewriter resolves from PRE-REWRITE identity. Plans get committed under session_B but the rewriter reads from session_A.

**Evidence chain**:
1. `test_h9_thread_identity_diverges_after_early_turn_removal` — PASSES (identities diverge)
2. `context_api.rs:243` — MCP falls back to `active_session_id()` when no explicit session
3. `ingest.rs:26` — `ensure_session()` with POST-REWRITE identity sets active session
4. Control test: middle-turn removal preserves identity (anchors intact)

**Fix 1 (DONE)**: Pass PRE-REWRITE thread_identity through capture exchange to ingest. `capture.rs:set_thread_identity()` + `handler.rs:471-481`.
**Fix 2 (DONE)**: Guard breadcrumb on `pending_plan.is_some()` — re-application shouldn't breadcrumb. `planner/mod.rs:547+651`.
**Fix 3 (DONE)**: Turn-aware MCP tool stripping. "Last assistant message" boundary — strip stale MCP tools, preserve recent. All 3 API formats. `planner/cleanup.rs`.

712 tests passing (659 Rust + 53 frontend), clippy clean.

### R14 Manual Test (2026-02-23) — VERIFIED
- **4+ cleans all fired correctly** — plan layering regression RESOLVED
- Plans stacked across turns, all stayed stripping, session continued fine
- `/context` Messages row shows cumulative JSONL size (220k+) while budget % is correct (50%)
  - This is expected: JSONL holds full history, Aperture strips ~200k per turn via proxy
  - Useful signal for archival — JSONL preserves everything
- **New bug**: Parallel MCP calls crash the proxy (connection refused on port 5400)
  - Triggered when Claude fires status + 2 searches simultaneously
  - Proxy auto-restarts, engine reloads from SQLite, but brief partial-reload window
  - Fix: serialize Aperture MCP calls (tokio Mutex queue) or handle concurrency in engine
- **New observation**: Claude pauses after MCP tool calls, waits for "continue" prompt
  - Happens after plan/stage/commit — says it will do something, then stops
  - May be related to tool_result format or missing continuation signal
- **Improvement idea**: Block ID display aliases (B1, B42 instead of hex UUIDs)
  - Reduces friction for manual management
  - Status truncates at ~86 blocks — search needed for large sessions
  - Keep UUIDs internally, expose aliases in preview/status

### R11 (2026-02-21, during hackathon demo filming)
- Rounds 1+2 fired correctly (4+5 = 9 blocks archived, persisted across turns)
- Round 3 (13 blocks) committed successfully (MCP returned success) but NEVER fired
- `/context` kept growing (conversation overhead outpaced archival) confirming blocks NOT removed
- Breadcrumb never showed round 3 additions

### R12 (2026-02-21)
- 2 successful cleans, 3rd round failed (same pattern as R11)

### Hypotheses for 3rd-round failure
- **H3**: `add_persistent_archives_for_session()` replaces existing list instead of merging on 3rd call
- **H4**: Block IDs in 3rd plan don't match blocks currently in session store (stale IDs after 2 rounds of archival)
- **H5**: Session ID diverges after multiple plan cycles (accumulation of state drift)
- **H6**: Capacity/size issue in persistent_archived_ids that manifests at 3+ rounds

### What's Fixed (Cumulative)

**Rounds 1–4 (P0 blockers):**
- **CRITICAL-2**: `is_context_tool_name()` matches MCP-namespaced tool names
- **CRITICAL-1**: Turn-aware projection + partial-turn stubs in all 3 API formats
- **MEDIUM-1**: Model-aware session flip guard prevents Haiku from stealing active session
- **Cache root cause**: Manifest removed, threshold warnings only, batch-gated heuristics
- **Orphan tool_use sanitizer**: Prevents 400 errors from orphan tool_use after cleanup/archival
- **Session affinity**: Plan stage/commit/append carry session hints through MCP
- **Refactor**: 3 tranches + hygiene pass complete (see `docs/REPO_STRUCTURE.md`)

**Rounds 5–6 (rewriter/cleanup pipeline):**
- **BUG #1**: `sanitize_anthropic_message_structure()` — merge consecutive roles, ensure user-at-end
- **BUG #3**: `serde_json` `preserve_order` feature — thinking block key order preserved
- **BUG #5**: Billing header filter before `content_fingerprint()` — stable system block IDs

**Round 8 (thinking block corruption — 3 mechanisms + 3 additional bugs) — ALL VERIFIED R9:**
- **F2**: Guard thinking/redacted_thinking blocks in `replace_content_block_with_stub()` — early return
- **F3**: Exclude `Role::Thinking` from archival candidates + validation rejection
- **F1**: Pipeline reorder: stubs → replacements → removal (indices correct before shifts)
- **F4**: Skip merge for consecutive assistant messages with thinking blocks (insert synthetic user)
- **Fix B**: Filter context tool blocks from engine ingest (don't accumulate MCP tool blocks)
- **F6**: Tokenized multi-word search (split query into terms, score each independently)
- **F5**: Unknown plan parameter detection with helpful error listing expected params

### Round 10 Manual Test (2026-02-20) — PASSED

- All 5 MCP tools confirmed working
- All 6 plan operations confirmed (archive, compress, expand, recall, pin, shift_to)
- Persistent archival stacking: 3 rounds accumulated correctly (8+8+5 = 21 blocks stripped)
- Plans layered with user turns between them fire correctly
- Remaining low-severity bugs: breadcrumb delta +0, budget % gap (~17.5% from overhead)
- Observation: each plan cycle costs 2-3k tokens in tool overhead — target blocks >3k for ROI

### R9-1/MT-1: Plan Layering Failure (P0) — FIX IMPLEMENTED

**Problem**: Second/third committed plans never fire. Only first plan's 8-block archival persists.

**Root cause confirmed**: `commit_staged_plan_for_session()` only sets `pending_plan` — does NOT update `persistent_archived_ids`. Archive IDs only persist when `plan_for_session()` runs with a pending plan. If the pending plan is never consumed (runtime condition TBD), the archive IDs never persist.

**JSONL evidence**:
- Plan1 commits + fires correctly (breadcrumb shows Net: -45k, Budget: 49%)
- Plan2 commits (MCP returns "Committed staged plan — 10 mutations")
- But breadcrumb after plan2 commit shows ONLY plan1's 8 blocks (Net: +0, Budget: 27%)
- Plan2's target blocks remain ACTIVE in engine (confirmed by preview)

**Exhaustive static analysis verified** (Rounds 10 + 10b):
- MCP affinity → context_api → planner commit: all use correct session ✓
- `commit_staged_plan_for_session()` correctly sets pending_plan ✓
- No code clears pending_plan between commit and consumption ✓
- Thread identity stable — `tool_result` gets `Role::ToolResult`, not `Role::User` ✓
- Block IDs stable — content-fingerprint-based, position-independent ✓
- POST-REWRITE capture doesn't change thread identity ✓
- Streaming race condition window exists but too narrow for persistent failure ✓

**Runtime root cause (needs tracing)**:
- H1: Session mismatch (static paths all resolve to S1, but something diverges at runtime)
- H2: Streaming response race (ingest remove/insert window could cause cold-start path)
- 3 `warn!()` calls will definitively distinguish H1 vs H2

**Fix**: Option B — add archive IDs to `persistent_archived_ids` at commit time.
Works regardless of whether root cause is H1 or H2.

**Implementation (2026-02-20)**:
- `engine/planner/mod.rs:237-262` — `add_persistent_archives_for_session()` method
- `metacog/tools/plan.rs:248` — Called after `commit_staged_plan_for_session()`
- 4 diagnostic `warn!()` calls: rewriter cold-start, rewriter consume, context_api, planner
- `mcp/server.rs:64-89` — Retry loop for `call_proxy()` (2 attempts, 500ms)
- 3 new tests for persistent archive behavior

## R16 Deep Dive (2026-02-24) — DIAGNOSTICS IMPLEMENTED

**Exhaustive code audit of ALL hot paths found ZERO panic sources in application logic.**

### What Was Done
1. **Full audit**: Every function in the concurrent path verified panic-safe: `dispatch_tool_with_limits_for_session`, `ingest()`, `session_blocks()`, `session_budget_status()`, `CaptureStore::finalize_streaming()`, `DynDispatcher::emit()`.
2. **16 additional `.expect()` on Mutex locks FIXED**: RunawayGuard ×3, ActionLog ×7, CompressionQueue ×6 — all replaced with `.ok()` fallbacks. These were missed in R15 and could cascade if any lock poisoned.
3. **Crash diagnostics IMPLEMENTED**:
   - `std::panic::set_hook()` → `/tmp/aperture-crash.log` (ALL threads)
   - `catch_unwind` around proxy thread
   - `RUST_BACKTRACE=1` set programmatically
   - Diagnostic `warn!()` in `finalize_exchange` + SSE task (ingest timing)
4. **Key insight**: MCP server is sequential (blocking stdin loop) — proxy never sees concurrent `/_aperture/` requests. Semaphore is never contended.

## R17 Deep Dive (2026-02-24) — ROOT CAUSE CONFIRMED + QUICK FIX

**Root cause: WebKitGTK crashes → tao calls `std::process::exit(0)` → all threads die instantly.**

### Evidence Chain
1. **No `/tmp/aperture-crash.log`** — global panic hook never fired. Not a Rust panic.
2. **No dmesg OOM entries** — not memory pressure.
3. **`coredumpctl list`** — **20 WebKitWebProcess crashes in 6 days** (Feb 18–24), all SIGABRT in libc malloc internals. Chronic WebKitGTK instability.
4. **Timestamp correlation** (system timezone MST = UTC-7):
   - MT1 session death: 12:09 MST → WebKit coredump: **12:10:37 MST** (1 min later)
   - MT2 session death: ~12:56 MST → WebKit coredump: **12:58:33 MST** (2 min later)
5. **MT2 stack trace**: `abort()` → libc malloc assertion → `_dl_deallocate_tls` → `libgallium-25.3.5` (Mesa GPU driver) → `exit()`. Heap corruption in GPU TLS cleanup.

### The Kill Chain (traced through Tauri/tao/wry source)
```
WebKitWebProcess crashes (SIGABRT, heap corruption in Mesa/libc)
  → WebKitGTK fires web-process-terminated signal
  → tao receives TaoWindowEvent::Destroyed
  → Window list becomes empty (Aperture has 1 window)
  → RunEvent::ExitRequested fires — Aperture has NO handler (lib.rs:510 uses default)
  → ControlFlow::Exit set (nothing calls api.prevent_exit())
  → tao's EventLoop::run() calls std::process::exit(0)
  → ALL threads killed instantly — no unwinding, no destructors, no crash log
  → Port 5400 → "connection refused"
```

**Key source locations in dependencies:**
- `tauri-runtime-wry` `lib.rs:4171-4186` — Destroyed → empty windows → ExitRequested
- `tao` `platform_impl/linux/event_loop.rs:979-984` — `process::exit(exit_code)`
- Aperture `lib.rs:510` — `.run(tauri::generate_context!())` (no exit handler)

### Quick Fix (IMPLEMENTED — partially effective)
Changed `Builder::run()` → `Builder::build()` + `App::run()` with callback:
- Tracks user-initiated close via `bool` (set on `WindowEvent::CloseRequested`)
- On `ExitRequested`: if user close → allow exit; if WebKit crash → `api.prevent_exit()`
- `lib.rs:510-580`

**MT3 (parallel MCP) — FIXED**: 3+4 concurrent MCP searches all survived. Quick fix effective.
**MT4 (file edit) — NOT FIXED**: README.md edit → WebKit coredump at 14:08:46 MST (PID 2749, identical SIGABRT/libgallium stack). App died — exit path bypassed handler entirely (no trace log written).
**MT5 (file edit, with tracing)** — **NOT a crash**: exit trace shows clean sequence:
```
CloseRequested(user_close=true) → Destroyed → ExitRequested(allowed) → Exit
```
No coredump, no panic. The window received a `CloseRequested` event that our handler interpreted as user-initiated close.

### Two Separate File-Edit Death Mechanisms
1. **WebKit crash (MT4)**: SIGABRT in libgallium/Mesa. Kills process before ExitRequested fires. Our handler is irrelevant — the process is dead before it gets a chance.
2. **Vite HMR reload (MT5)**: Vite's chokidar watches the project root (only ignores `**/src-tauri/**` per `vite.config.js:52-54`). Any non-Rust file change (README.md, docs/*.md) triggers a full-page reload signal to the webview → `CloseRequested` → clean exit. Our handler allows it because `user_close=true`.

Both are resolved by the proper fix (proxy as separate process — immune to both WebKit crashes and Vite restarts).

### Open Questions
- **Why does Vite full-page reload kill the Tauri app?** A Vite HMR reload should just refresh the webview content, not close the window. Possible: WebKitGTK handles the reload poorly, or `tauri dev` interprets the reload as a restart signal.
- **MT1 24-minute gap**: Proxy died at 11:46 MST but WebKit coredump at 12:10 MST. Suggests proxy died independently or WebKit partially failed first.
- All questions mooted by the proper fix (proxy as separate process).

### Proxy Decoupling — IMPLEMENTED (2026-02-24)
Proxy decoupled from Tauri's process lifecycle:
- `aperture-proxy` binary owns engine + proxy (separate PID, survives all Tauri/WebKit crashes)
- Tauri spawns it as detached process (setsid on Unix), connects via HTTP/SSE
- `BroadcastDispatcher` → `tokio::sync::broadcast` → SSE at `/_aperture/events`
- `/_aperture/ipc/{command}` — HTTP IPC replaces all Tauri engine/hot-patch commands
- Frontend uses `invokeProxy()` (fetch) + `EventSource` instead of Tauri `invoke()`/`listen()`
- Terminal stays as Tauri IPC (PTY is process-local)
- CORS headers on all `/_aperture/` responses for webview cross-origin access
- 667 Rust tests + 53 frontend tests passing, clippy clean

**RESULT**: UI connects to proxy successfully. Proxy survives Tauri window close.
**BUT**: File edits through `aperture claude` still cause the UI to disconnect/crash.
The proxy process itself stays alive (confirmed healthy on port 5400 after crash), but
the Tauri webview dies and the frontend loses its SSE connection.

### File-Edit Crash — RESOLVED (2026-02-24)

**Root cause confirmed**: Tailwind v4's oxide scanner puts `.md` files into Vite's module graph
via `addWatchFile()`. When any markdown file changes, Vite triggers `{ type: "full-reload" }`
because `.md` is a non-CSS file with only CSS importers (HMR dead end). The full-page reload
either (a) refreshes the webview cleanly or (b) triggers a WebKitGTK SIGABRT in Mesa/libgallium
during GPU repaint.

**Evidence chain**:
1. `coredumpctl list` — 20 WebKitWebProcess SIGABRT in 6 days, all in libgallium heap corruption
2. Tailwind oxide scanner includes `.md` in `template-extensions.txt` → `addWatchFile()` → module graph
3. `vite:css-analysis` links `.md` files as CSS module dependencies → `getModulesByFile()` finds them
4. HMR propagation: non-CSS, non-SVG with only CSS importers = dead end → `needFullReload = true`
5. `vite.config.js` only ignored `**/src-tauri/**` — all other files triggered the chain

**Fix applied**:
- Expanded `server.watch.ignored` in `vite.config.js` to cover `**/*.md`, `**/dev/**`, `**/docs/**`,
  `**/.claude/**`, `**/.context/**`, `**/target/**`
- Chokidar no longer watches these paths → no module graph invalidation → no reload
- Verified: markdown edits from within Aperture project dir no longer crash UI

**Remaining WebKitGTK instability**: Mesa/libgallium SIGABRT can still occur spontaneously
(not triggered by file edits). Proxy decoupling handles this — engine/proxy survive all
Tauri/WebKit crashes. Updating Mesa drivers is the real fix (outside Aperture's scope).

**Session lifecycle UX** (also fixed this session):
- App starts with clean slate (no stale blocks from previous session)
- Blocks clear ~15s after client exits (idle timeout fires session reset)
- New session blocks populate on first `context_updated` event

## Remaining Work (Priority Order)

1. ~~Implement R9-1 Option B fix~~ — **DONE**
2. ~~Add diagnostic tracing~~ — **DONE**
3. ~~Add MCP call_proxy retry~~ — **DONE**
4. ~~Manual test Round 10~~ — **PASSED**
5. ~~Fix 1 (session divergence)~~ — **DONE**
6. ~~Fix 2 (breadcrumb guard)~~ — **DONE**
7. ~~Fix 3 (MCP tool stripping)~~ — **DONE**
8. ~~Manual test Round 14~~ — **PASSED (4+ cleans, regression resolved)**
9. ~~Analyze R14/R15 logs~~ — **DONE (R17: root cause = WebKitGTK → process::exit)**
10. ~~Fix parallel MCP crash (quick fix)~~ — **DONE (ExitRequested handler, MT3 confirmed)**
11. ~~Proxy decoupling~~ — **DONE (2026-02-24)** — Separate `aperture-proxy` process
12. ~~File-edit crash~~ — **DONE (2026-02-24)** — Vite ignore patterns + session idle clearing
13. **Block ID display aliases** — B1/B42 style aliases mapped to UUIDs
13. **Fix breadcrumb delta bug** — low severity, delta shows +0 on re-archival
14. **Fix budget % gap** — include overhead in engine budget calculation
15. **Fix D: Cache + Archival Death Spiral** — cache-aware archival strategy
16. **P1: Economics Ledger** — token cost instrumentation
17. **P3: Schema Overhead Reduction** — consolidate tools, lazy injection

See `tasks.md` for full checklist with subtasks.

## Architecture Ownership Map

| Module | Owns | Must Not |
|--------|------|----------|
| **Parser** (`proxy/parser/*`) | Wire parsing → canonical `Block` records, thread identity, overhead estimation | Mutate engine state or apply policy |
| **Rewriter** (`proxy/rewriter/*`) | JSON mutation, runtime cleanup, trailing injection | Decide archival/compression policy |
| **Planner** (`engine/planner/*`) | Mutation planning, staged plans, heuristics, persistent archive intent | Patch provider JSON directly |
| **Engine** (`engine/`) | Session/block state, ingest, persistence, policy-enforced mutations | Parse provider wire formats |
| **Handler** (`proxy/handler/*`) | Upstream routing, transport filtering, flow orchestration | Own provider JSON transformation |
| **Interceptor** (`proxy/interceptor/*`) | Context-tool interception, bounded reinvoke | Own session state or planner policy |
| **Capture** (`proxy/capture/*`) | Capture store lifecycle, SSE reconstruction | Own session policy or rewrite decisions |
| **MCP** (`mcp/*`) | JSON-RPC transport, tool routing, session affinity forwarding | Own planner semantics or mutation policy |

## Key Constraints

### Cache Economics
- **Anthropic**: Cache hierarchy tools→system→messages. Cumulative hashes. 1.25× write, 0.1× read. Max 4 breakpoints.
- **OpenAI**: Fully automatic. Free write, 50-90% read discount. 1024 min cacheable.
- **Both**: Tool/system changes invalidate from that point onward.

### Stateless Clients
All major LLM coding tools send full conversation history each request. Aperture must re-apply archive mutations every turn to keep forwarded prefix stable.

### API Invariants
- Every `tool_use` needs a `tool_result` (Anthropic)
- Non-empty content blocks required
- Turn alternation (user/assistant) must be maintained
- Partial-turn stubs preserve these invariants; full-turn removal is safe

## P0 Mitigations (Preserved)
- Argument validation, output size caps (8KB normal, 2KB compact)
- Proxy runaway guard (rolling window, fail-open)
- Circuit breaker (60s lockout on 24+ calls/60s)
- Kill switch (`APERTURE_CONTEXT_TOOLS_MODE=passive|disabled|off`)
- Orphan sanitizers (both directions)
- Deterministic block IDs, staged planning controls

## Diagnostic History

| Round | Report | Key Finding |
|-------|--------|-------------|
| 1–2 | `deep-dive-diagnostics-round-{1,2}-2026-02-19.md` | Projection mismatch, cleanup naming, session flips |
| 3 | `deep-dive-diagnostics-round-3-2026-02-19.md` | Elevated CRITICAL-1, confirmed round-2 findings |
| 4 | `deep-dive-diagnostics-round-4-consolidated-2026-02-19.md` | Cascading failure chain proven, all fixes designed |
| **5** | **`deep-dive-diagnostics-round-5-2026-02-19.md`** | **3 new P0/P1 bugs in rewriter/cleanup pipeline** |
| **10** | **`deep-dive-diagnostics-round-10-2026-02-19.md`** | **R9-1 root cause: persistent_archived_ids gap at commit. Option B fix confirmed.** |
| **R10-MT** | **RESUME.md inline (2026-02-20)** | **Best run — 2 successful cleans, all tools/ops verified, 3 fixes implemented** |

Manual test logs in `~/.claude/projects/-home-caden-projects-Aperture/`:
- **MT1/R17 (2026-02-24)** `3fef6a4a...` — 3 parallel + 2 parallel both crashed. WebKit coredump at 12:10 MST (PID 110966, SIGABRT). File edit crash reproduced at session end.
- **MT2/R17 (2026-02-24)** `02fae9ca...` — 3 parallel survived, 4 parallel crashed (2 ok, 2 fail). WebKit coredump at 12:58 MST (PID 191047, SIGABRT in libgallium/Mesa TLS). No Rust panic log. Root cause confirmed: WebKitGTK → process::exit(0).
- **MT3/R17 (2026-02-24)** — Quick fix compiled. 3+4 parallel MCP searches ALL survived (fix works for parallel). Claude edited `lib.rs` → app restarted (expected: Tauri dev watcher hot-reload on Rust source change, not a bug).
- **MT4/R17 (2026-02-24)** — README.md edit. WebKit coredump at 14:08:46 MST (PID 2749, SIGABRT/libgallium). No exit trace written — process died before handler fired.
- **MT5/R17 (2026-02-24)** — File edit with exit tracing. Clean exit: `CloseRequested → Destroyed → ExitRequested(allowed) → Exit`. No coredump. Vite HMR full-page reload on non-source file change → window closed. Two file-edit death mechanisms confirmed: (a) WebKit crash (MT4), (b) Vite HMR reload (MT5).
- **R15 (2026-02-24)** `3fef6a4a...` — Parallel MCP crash fix test. Fix WAS compiled (confirmed via `strings`). Crash reproduced 2× (3 parallel, 2 parallel) despite semaphore. File edits worked. Crash is below handler level.
- **R14 (2026-02-23)** `a9cf1a72...` ("Yooo") — 3 plan cycles worked. 2 parallel MCP crashes. UUID hallucination. `267a1c72...` ("whats up") — 4 plan cycles worked. No crash (only 1 parallel pair). Pausing pattern.
- **Round 10 manual test (2026-02-20)** — TBD (find in session logs)
- `5a933896...` — Round 9 manual test (Sonnet 4.6, 87 turns)
- `db654aac...` — Round 5 manual test ("whats poppin claude")
- `1baf6b88...` — Pre-fix ("Yoo claude")
- `df4ad515...` — Pre-fix ("whats up claude")
- `66dd683a...` — Fresh repro ("claude!")

## Proxy Decoupling Architecture (Proper Fix)

### Why Decouple
The proxy and engine have zero dependency on Tauri/WebKit. They share a process only because of initial convenience. When WebKitWebProcess crashes (chronic, 20× in 6 days on Linux), `tao` calls `process::exit(0)` and kills the proxy as collateral damage. The quick fix (ExitRequested handler) prevents this, but the proxy is still vulnerable to Tauri main process crashes.

### Current Coupling Points
1. **`DynDispatcher`** — type-erased `Arc<dyn Fn(&ApertureEvent) + Send + Sync>`. Currently wraps `app_handle.emit()`. Only 6 call sites: 5 in proxy handler (request/response/blocks/error events), 1 in engine (`context_updated`).
2. **29 `#[tauri::command]` functions** in `lib.rs` (lines 114-369) — direct `Arc<ContextEngine>` method calls via Tauri IPC.
3. **`app.manage(engine)`** — engine registered as Tauri managed state.
4. **`std::thread::spawn`** in `.setup()` — proxy thread lifetime bound to Tauri process.

### What's Already Decoupled
- **ContextEngine**: Zero `use tauri` imports in entire `engine/` directory. Works standalone with `new(None)`.
- **`start_proxy()`**: Takes `Option<DynDispatcher>` and `Option<Arc<ContextEngine>>`. Tested without either.
- **`/_aperture/` HTTP API**: Already exposes full engine (preview, read, search, plan, status, health). Tauri frontend could use this instead of IPC.
- **Existing binary pattern**: `aperture-mcp` binary links `aperture_lib`, demonstrates standalone use.

### Implementation Plan
1. **New `src/bin/aperture_proxy.rs`** (~50 lines):
   - Init logging, create `ContextEngine::new(None)` (or with SSE dispatcher)
   - Call `proxy::start_proxy(port, dispatcher, None, Some(engine))`
   - Add `/_aperture/events` SSE endpoint for real-time event push
2. **Tauri app changes**:
   - Remove `std::thread::spawn` proxy thread from `.setup()`
   - Launch `aperture-proxy` as child process (or Tauri sidecar)
   - Replace 29 `#[tauri::command]` IPC calls with `fetch("http://localhost:5400/_aperture/...")`
   - Subscribe to `/_aperture/events` SSE stream for real-time updates
3. **DynDispatcher replacement**:
   - Standalone proxy: events pushed to SSE subscribers
   - Tauri frontend: EventSource listener on `/_aperture/events`
   - No more `app_handle.emit()` — events flow over HTTP

### Migration Strategy
- Phase 1: Quick fix (ExitRequested handler) — immediate crash protection
- Phase 2: Add `/_aperture/events` SSE endpoint to existing proxy — no breaking changes
- Phase 3: New `aperture-proxy` binary, migrate Tauri IPC to HTTP, remove proxy thread
- Each phase is independently shippable and testable
