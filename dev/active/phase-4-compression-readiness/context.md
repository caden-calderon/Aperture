# Phase 4 Token Economics Context

## Current State (2026-02-20)

**Manual test Round 10 PASSED.** 2 successful context cleans. All 3 fixes (Option B, diagnostic tracing, MCP retry) implemented. 696 tests passing (643 Rust + 53 frontend), clippy clean. Log analysis pending for H1/H2 root cause confirmation.

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

**Analyze Round 10 JSONL logs:**
1. Grep for `R9-DIAG` in proxy logs — compare session IDs across trace points
2. Confirm H1 vs H2 root cause
3. Assess breadcrumb delta bug fix approach
4. Downgrade diagnostic tracing to `debug!()` after confirmation

## Remaining Work (Priority Order)

1. ~~Implement R9-1 Option B fix~~ — **DONE**
2. ~~Add diagnostic tracing~~ — **DONE**
3. ~~Add MCP call_proxy retry~~ — **DONE**
4. ~~Manual test Round 10~~ — **PASSED (2 successful cleans)**
5. **Check diagnostic logs** — confirm H1 vs H2
6. **Fix breadcrumb delta bug** — low severity, delta shows +0 on re-archival
7. **Fix budget % gap** — include overhead in engine budget calculation
8. **Fix D: Cache + Archival Death Spiral** — cache-aware archival strategy
9. **P1: Economics Ledger** — token cost instrumentation
10. **P3: Schema Overhead Reduction** — consolidate tools, lazy injection

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
