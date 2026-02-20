# Deep Dive Diagnostics — Round 7

**Date**: 2026-02-19
**Session**: `70b1ff19-914d-43ed-ad1f-8520729e8ac5` ("whats good claude", Opus 4.6)
**Duration**: ~7 minutes (23:47:29 – 23:54:35 UTC)
**Previous round**: Round 6 fixes (BUGs #1-3, #5) all working — zero API 400 errors

---

## Executive Summary

Round 6 fixes eliminated all 400 errors. The proxy now successfully handles archival (first clear: -31k tokens confirmed). However, a **critical design flaw** in the context tool cleanup pipeline causes **model amnesia** — the model's context tool calls are silently stripped between turns, making it unable to remember previous interactions. This triggers an infinite preview loop, catastrophic cache invalidation, and massive token waste.

**Cost impact**: $6.56 wasted out of $14.61 total (44.9%) in a 7-minute session.

---

## Session Timeline (Abbreviated)

| Phase | Time | What Happened | Outcome |
|-------|------|---------------|---------|
| Fill context | 23:47-48 | Bulk-read 11 source files | Hit 61% (122k/200k) |
| First clear (30k) | 23:49-50 | preview → plan(stage) → plan(commit) | **SUCCESS**: 61% → 45% |
| Verify | 23:50-51 | /context confirms 91k/200k | ✓ Working |
| Second fill | 23:51-52 | Read 4 more files | Hit 58% (117k/200k) |
| Second clear (50k) | 23:52-54 | **LOOP**: 15+ preview calls, never calls plan | **FAILURE**: Context GREW to 60% |
| User gives up | 23:54:35 | 4 manual rejections | Session abandoned |

---

## Bugs Found

### BUG R7-1: Context Tool Stripping Causes Model Amnesia (P0 — CRITICAL)

**Root cause**: `strip_anthropic_context_tools()` (cleanup.rs:88-186) strips ALL `aperture_context_*` AND `mcp__aperture__aperture_context_*` tool_use + tool_result blocks from the conversation on every request. This runs via `runtime.cleanup_history()` at rewriter.rs:184.

**Flow**:
```
Turn N:   Model calls mcp__aperture__aperture_context_preview
          Claude Code gets MCP result, includes tool_use + tool_result in Turn N+1

Turn N+1: Proxy receives request with aperture tool blocks in messages[]
          → Parser ingests blocks into engine (blocks exist in engine)
          → Rewriter calls cleanup_history()
          → strip_anthropic_context_tools() DELETES the tool_use + tool_result
          → Anthropic receives payload with NO evidence the tool was ever called
          → Model has zero memory of calling preview
          → Model calls preview AGAIN → infinite loop
```

**Why the first clear worked but the second didn't**: The first clear succeeded in a single "chain" of tool calls within ~30 seconds. The model called preview → saw ~31k in large blocks → immediately called plan(stage) → plan(commit). It didn't need to remember across turns because it chained the calls quickly.

The second clear failed because the model saw only ~6-8k in suggestions (far short of 50k goal). It called preview, got insufficient results, and on the next turn — having lost memory of the previous preview — called preview again.

**The `is_context_tool_name()` function** (runtime.rs:66-68) matches both:
- Bare prefix: `aperture_context_*` (used by interceptor/proxy-injected tools)
- MCP prefix: `mcp__aperture__aperture_context_*` (used by Claude Code MCP)

**These should NOT be treated the same.** MCP tools are legitimate conversation entries managed by Claude Code. Stripping them erases the model's working memory.

**Breadcrumb gap**: The breadcrumb system (cleanup.rs:20-61) only generates summaries for MUTATIONS (archive/expand/shift). For read-only operations (preview, search, status), no breadcrumb is generated. The tool calls are stripped with zero trace left behind.

**Evidence from JSONL**:
- Lines 117-200: 15+ consecutive preview calls, model never progresses to plan
- Line 145: Model says "Those were already archived from earlier" — confusion from stripped history
- Line 195: Model says "same 3 blocks keep getting re-stripped" — can only see breadcrumb from first clear, not its own recent previews
- User manually rejected tool use 4 times (lines 160, 162, 186, 202)

---

### BUG R7-2: Cache Invalidation from Tool Stripping (P0 — CRITICAL)

**Root cause**: Stripping context tool blocks from the messages array changes the cumulative hash at the removal point. All subsequent content is a cache miss.

**Anthropic cache hierarchy**: `hash(block_N) = hash(block_0, ..., block_N)`. Removing or modifying any block invalidates everything after it.

**Evidence from JSONL token data**:

| Turn Type | Uncached Input | Cache Read | Cache Hit Rate |
|-----------|---------------|------------|----------------|
| Normal (no context tools) | 1-101 | 118k-130k | **99%+** |
| Post-strip (context tools stripped) | **90k+** | 28k | **23%** |

The 28k of cache read on stripped turns = system prompt + tool definitions (stable prefix). Everything else (90k+ tokens of messages) is uncached.

**Quantified waste**:
- 16 loop turns with cache misses
- 1,457,559 uncached input tokens from loop (83.5% of ALL uncached input)
- 450,496 cache-read tokens during loop (only the stable prefix)
- **$6.56 wasted** ($7.51 actual loop cost vs $0.95 if fully cached)
- **44.9% of total session cost** ($14.61) attributable to the loop

**Impact on 5-hour session limit**: The entire 8M tokens consumed in 7 minutes represents a non-trivial fraction of the session budget. In a real work session, this loop could exhaust limits rapidly.

---

### BUG R7-3: Block Count Inflation from MCP Tool Interactions (P1)

**Root cause**: Each MCP context tool call generates tool_use + tool_result blocks that get ingested into the engine (parser runs BEFORE rewriter). These accumulate without bound.

**Evidence**: Block count over the session:
```
Turn 55:  41 blocks  (pre-archive)
Turn 62:  46 blocks  (+5 from first preview chain)
Turn 120: 78 blocks  (post-fill + tool overhead)
Turn 126: 82 blocks  (+4 from two previews)
Turn 130: 84 blocks  (+2)
Turn 153: 88 blocks  (+4)
Turn 174: 92 blocks  (+4)
Turn 199: 102 blocks (+10 from continued looping)
```

Block count grew from 41 to 102 (149% increase). Engine reports "55% budget (111k used)" but the actual API payload is different because context tool blocks are stripped. This creates a **disconnect between engine view and API reality**.

---

### BUG R7-4: Archival Suggestions Insufficient for Large Goals (P2)

**Root cause**: The heuristics only suggest stale Middle-zone blocks for archival. Recent blocks (Recency zone) are never suggested, even when they contain large tool results (file reads).

**Evidence**:
- First clear: Preview showed 3 large tool_result blocks (~31k total) → model could achieve 30k goal ✓
- Second clear: Preview showed 20-40 small blocks (~6-8k total) → model could NOT achieve 50k goal ✗
- The large file-read results from Phase 4 were in Recency zone (recent turns) and not eligible for archival suggestions
- Model saw `max_savings_available = ~6-8k` vs `target = 50k` → could never succeed → kept retrying

---

### BUG R7-5: Context Grows Instead of Shrinking During Clear Attempts (P1)

**Root cause**: Each preview call adds ~2-4 new blocks (tool_use + tool_result + text) to the conversation. Over 15 preview calls, the model ADDED ~24 blocks while trying to REMOVE context.

**Evidence**:
- Pre-clear: 117k/200k (58%)
- Post-loop: 119k/200k (60%) — context GREW by 2k despite the clear attempt
- Block count: 78 → 102 (+24 blocks of tool interaction overhead)

This is a vicious feedback loop: attempting to clear context makes context bigger.

---

### BUG R7-6: UserPromptSubmit Hook Errors (P3 — Cosmetic)

**Observed**: "UserPromptSubmit hook error" visible in the screenshot at session start.

**Analysis**: These appear in the Claude Code shell output but were NOT present in the JSONL conversation log. Likely a hook misconfiguration in the user's `.claude/` settings rather than an Aperture bug.

---

## Root Cause Relationships

```
R7-1 (AMNESIA) ──→ R7-5 (CONTEXT GROWS)
     │                  ↑
     │           R7-3 (BLOCK INFLATION)
     │
     └──→ R7-2 (CACHE DEATH) ──→ TOKEN WASTE ($6.56 in 7 min)

R7-4 (INSUFFICIENT SUGGESTIONS) ──→ Model can't achieve goal ──→ keeps retrying
                                          ↑
                                     R7-1 (AMNESIA) strips memory of previous attempts
```

R7-1 is the root of the cascading failure. Fixing it eliminates R7-2, R7-5, and the loop behavior. R7-3 and R7-4 are independent issues that should be addressed separately.

---

## Fix Recommendations

### Fix A: Stop Stripping MCP Context Tools (Fixes R7-1 + R7-2)

**The fix**: Modify `is_context_tool_name()` or the strip functions to differentiate between:
- **Bare prefix** `aperture_context_*` → **STRIP** (interceptor/proxy-injected tools, not in Claude Code's conversation)
- **MCP prefix** `mcp__aperture__aperture_context_*` → **DO NOT STRIP** (Claude Code MCP tools, part of legitimate conversation)

**Location**: `src-tauri/src/metacog/runtime.rs:66-68` (the matching function) OR `src-tauri/src/engine/planner/cleanup.rs:88+` (the strip functions)

**Rationale**:
- MCP tools are managed by Claude Code — it expects them in the conversation
- The model NEEDS to see its previous tool calls to avoid re-calling
- Keeping them avoids cache invalidation (no message array mutations)
- Token cost of keeping tool calls (~2-3k per call) is FAR less than cache misses (~90k per stripped turn)

**Option A1**: New function `is_intercepted_context_tool_name()` that only matches bare prefix. Use this in strip functions, keep `is_context_tool_name()` for other uses (tool interception, etc.).

**Option A2**: Add a parameter to strip functions: `strip_mcp: bool`. Default false for the Claude MCP runtime.

**Preferred**: Option A1 — cleanest separation of concerns.

### Fix B: Don't Ingest Stripped Tool Blocks Into Engine (Fixes R7-3)

**The fix**: After parsing, filter out blocks that will be stripped before ingesting into engine. Or add a flag to the parser to skip context tool blocks.

**Alternative**: Add a dedup/eviction mechanism for tool interaction blocks. Context tool blocks are transient and shouldn't accumulate in the engine's block store.

**Location**: `src-tauri/src/proxy/handler/` (between parse and ingest) or `src-tauri/src/engine/ingest.rs` (filter on ingest)

### Fix C: Expand Archival Suggestions for Large Goals (Fixes R7-4)

**The fix**: When the model requests a specific token target (via plan tool), allow suggestions from Recency zone. Or add a "force" tier that suggests all non-system blocks regardless of zone/staleness.

**Alternative**: Expose Recency zone blocks in preview output with a separate "aggressive" section showing what COULD be archived if needed.

**Location**: `src-tauri/src/engine/planner/heuristics.rs` or `src-tauri/src/metacog/tools.rs` (preview output)

### Fix D: Compact Old Context Tool Calls (Enhancement, post-fix-A)

After Fix A stops stripping MCP tools, old context tool calls will accumulate. Add a post-processing step that replaces tool calls older than N turns with compact text summaries:

```
Original: [tool_use: aperture_context_preview] + [tool_result: "78 blocks, 55% budget..."]
Compact:  [text: "[Context check: 78 blocks, 55% budget (111k/200k), 20 archival candidates (~6k tok)]"]
```

This preserves the model's memory while reducing token overhead.

---

## Priority and Fix Order

| Priority | Bug | Fix | Effort | Impact |
|----------|-----|-----|--------|--------|
| **P0** | R7-1 + R7-2 | Fix A: Stop stripping MCP tools | Small | Eliminates loop, cache death, token waste |
| **P1** | R7-3 | Fix B: Don't ingest stripped blocks | Small | Corrects engine block count |
| **P1** | R7-4 | Fix C: Better archival suggestions | Medium | Enables larger clear goals |
| **P2** | R7-5 | Auto-fixed by Fix A | — | — |
| **P3** | R7-6 | Investigate hook config | Trivial | Cosmetic |
| **Enhancement** | — | Fix D: Compact old tool calls | Medium | Token savings over long sessions |

**Recommended implementation order**: Fix A → Fix B → Fix C → Fix D

---

## Token Usage Summary

| Metric | Value |
|--------|-------|
| Session duration | 7 minutes |
| Total tokens processed | 8,071,870 |
| Uncached input tokens | 1,744,538 |
| Cache read tokens | 5,859,163 |
| Cache create tokens | 466,394 |
| Output tokens | 1,775 |
| **Total estimated cost** | **$14.61** |
| Loop turns (cache misses) | 16 |
| Loop uncached input | 1,457,559 (83.5% of all uncached) |
| **Cost wasted by loop** | **$6.56 (44.9% of session)** |

---

## What IS Working (Confirmed)

1. **Round 6 fixes**: Zero API 400 errors in entire session ✓
2. **Archival execution**: First clear (-31k tokens) confirmed by /context ✓
3. **Persistent archival**: Re-sent blocks get re-stripped on subsequent requests ✓
4. **Stub replacement**: Archived 10k+ blocks replaced with 30-34 token stubs ✓
5. **serde_json preserve_order**: No thinking block corruption ✓
6. **System fingerprint stability**: Billing header filtering working ✓
7. **Cache performance on normal turns**: 99%+ cache hit rate when no stripping ✓
8. **MCP server**: All context tools respond correctly via MCP ✓

---

## Files Referenced

| File | Relevance |
|------|-----------|
| `src-tauri/src/engine/planner/cleanup.rs:88-186` | `strip_anthropic_context_tools()` — THE root cause function |
| `src-tauri/src/metacog/runtime.rs:57-68` | `CONTEXT_TOOL_PREFIX`, `MCP_CONTEXT_TOOL_PREFIX`, `is_context_tool_name()` |
| `src-tauri/src/metacog/claude_mcp.rs:120-122` | Calls `strip_anthropic_context_tools()` in `cleanup_history()` |
| `src-tauri/src/proxy/rewriter.rs:184` | Where `cleanup_history()` is called in the rewrite pipeline |
| `src-tauri/src/proxy/rewriter.rs:73` | Cold-start path also calls `cleanup_history()` |
| `src-tauri/src/engine/planner/cleanup.rs:20-61` | `generate_breadcrumb()` — only for mutations, not preview calls |
| `src-tauri/src/engine/planner/heuristics.rs` | Archival suggestion logic |
| `src-tauri/src/proxy/parser/anthropic.rs` | Block ingestion (runs before rewriter) |

---

## Appendix: Cache Miss Pattern

```
Line  Type         Uncached    CacheRead   CacheHit%
 2    normal       3           10,315      99.97%
 5    normal       3           34,053      99.99%
 20   normal       1           34,113      99.99%
 33   normal       1           63,068      99.99%
 45   normal       60          97,897      99.94%
 55   context_tool 3           122,175     99.99%  ← well-cached (first preview)
 59   POST-STRIP   96,107      28,156      22.7%   ← CACHE DEATH after stripping
 63   normal       3           124,113     99.99%  ← recovered on next normal turn
 70   normal       1           129,402     99.99%
 75   POST-STRIP   62,582      28,156      31.0%   ← CACHE DEATH after commit stripping
 85   normal       101         90,810      99.89%
 107  normal       160         94,733      99.83%
 117  context_tool 101         116,776     99.91%
 121  POST-STRIP   90,769      28,156      23.7%   ← LOOP STARTS: CACHE DEATH
 127  POST-STRIP   90,782      28,156      23.7%
 131  POST-STRIP   90,782      28,156      23.7%
 135  POST-STRIP   90,782      28,156      23.7%
 139  POST-STRIP   90,782      28,156      23.7%   ← 5 consecutive cache misses
 143  normal       101         118,699     99.92%  ← model emits text before next tool
 150  POST-STRIP   90,984      28,156      23.6%
 154  POST-STRIP   90,984      28,156      23.6%
 158  POST-STRIP   90,984      28,156      23.6%
 171  POST-STRIP   91,041      28,156      23.6%   ← 4 more cache misses
 175  normal       101         119,050     99.92%
 182  POST-STRIP   91,245      28,156      23.6%
 193  normal       101         119,204     99.92%
 200  POST-STRIP   93,412      28,156      23.2%   ← final loop turn
```

Every POST-STRIP turn shows ~90k uncached tokens and only ~28k cached (stable prefix). Normal turns show 99%+ cache hit. The pattern is unmistakable: stripping destroys the cache.
