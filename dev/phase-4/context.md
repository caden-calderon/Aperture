# Phase 4 Token Economics Context

## Current State (2026-02-23)

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

708 tests passing (655 Rust + 53 frontend), clippy clean.

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

## Next Action

**Analyze R14 manual test logs (both sessions) for remaining issues:**
1. "whats up sonnet" session (~1-2 days ago)
2. "Yooo" session (today, 2026-02-23)
3. Investigate: parallel MCP crash (race condition / concurrency), Claude pausing behavior
4. Document findings, plan fixes

## Remaining Work (Priority Order)

1. ~~Implement R9-1 Option B fix~~ — **DONE**
2. ~~Add diagnostic tracing~~ — **DONE**
3. ~~Add MCP call_proxy retry~~ — **DONE**
4. ~~Manual test Round 10~~ — **PASSED**
5. ~~Fix 1 (session divergence)~~ — **DONE**
6. ~~Fix 2 (breadcrumb guard)~~ — **DONE**
7. ~~Fix 3 (MCP tool stripping)~~ — **DONE**
8. ~~Manual test Round 14~~ — **PASSED (4+ cleans, regression resolved)**
9. **Analyze R14 logs** — parallel MCP crash, Claude pausing, other observations
10. **Fix parallel MCP crash** — serialize with tokio Mutex or add request queue
11. **Block ID display aliases** — B1/B42 style aliases mapped to UUIDs
12. **Fix breadcrumb delta bug** — low severity, delta shows +0 on re-archival
13. **Fix budget % gap** — include overhead in engine budget calculation
14. **Fix D: Cache + Archival Death Spiral** — cache-aware archival strategy
15. **P1: Economics Ledger** — token cost instrumentation
16. **P3: Schema Overhead Reduction** — consolidate tools, lazy injection

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
- **Round 10 manual test (2026-02-20)** — TBD (find in session logs) ← **ANALYZE NEXT**
- `5a933896...` — Round 9 manual test (Sonnet 4.6, 87 turns)
- `db654aac...` — Round 5 manual test ("whats poppin claude")
- `1baf6b88...` — Pre-fix ("Yoo claude")
- `df4ad515...` — Pre-fix ("whats up claude")
- `66dd683a...` — Fresh repro ("claude!")
