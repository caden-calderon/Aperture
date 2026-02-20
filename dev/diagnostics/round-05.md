# Deep-Dive Diagnostics Round 5 — Post-P0-Fix Manual Test Analysis

**Date**: 2026-02-19
**Session**: `db654aac-a155-4291-aae6-2cb1dfd20b31`
**Phrase**: "whats poppin claude"
**Model**: claude-opus-4-6 (200k context)
**Prerequisite fixes**: CRITICAL-2 (MCP cleanup), CRITICAL-1 (turn-aware projection + stubs), MEDIUM-1 (session flip guard)

---

## Executive Summary

The P0 fixes from the previous session are partially working — the first archival genuinely reduced API payload by ~60k tokens, and session flips from Haiku are no longer observed. However, **3 new P0/P1 bugs** were discovered that make the session unusable after archival:

1. **Rewriter produces invalid message sequence** (payload ends with assistant message → "does not support prefill" 400 error, 4 occurrences)
2. **Rewriter modifies extended thinking blocks** (Anthropic rejects modified thinking blocks, 1 occurrence)
3. **Trailing whitespace in rewritten assistant content** (API validation error, 1 occurrence)

These are all in the **rewriter/cleanup pipeline** and are likely related to how Aperture strips MCP tool calls from conversation history.

---

## Chronological Timeline

### Phase 1: Context Fill (L1–L55) — HEALTHY

| Line | Event | API Input Tokens | Cache Hit % |
|------|-------|-----------------|-------------|
| L2 | User: "whats poppin claude" | — | — |
| L3 | First API response | 33.5k | 58.5% |
| L8–L22 | Reads: engine/mod.rs, handler.rs, rewriter.rs, runtime.rs | 33.6k | 99.7% |
| L23–L35 | Reads: interceptor.rs, planner/mod.rs, tools.rs, types.rs | 62.4k | 53.9% |
| L36–L48 | Reads: applicator.rs, context_api.rs, session.rs, block.rs | 91.4k | 68.3% |
| L49–L51 | Assistant: "Hit 44% — pausing here" | 111.7k | 81.9% |
| **L55** | **/context: 112k/200k (56%)** | — | — |

**Assessment**: Clean. Files read, context grew steadily, cache performance normal.

---

### Phase 2: First Archival Attempt (L57–L88) — PARTIAL SUCCESS

| Line | Event | API Input Tokens | Cache Hit % |
|------|-------|-----------------|-------------|
| L57 | User: "Clear some context, try and clear 30k tokens" | — | — |
| L61 | `aperture_context_preview` #1 | 113.6k | 98.2% |
| L64 | Result: 45 blocks, 54% budget, 6 archival suggestions | — | — |
| L65 | `aperture_context_preview` #2 (duplicate same turn) | — | — |
| L68 | Result: **50 blocks** (grew from 45), 54% budget, 10 suggestions | — | — |
| L70 | `aperture_context_plan` — stage archive of 9 blocks | 118.9k | 95.5% |
| L73 | Result: "Plan staged — 9 mutations" | — | — |
| L75 | `aperture_context_plan` — commit | 119.7k | 99.4% |
| L78 | Result: "Committed staged plan — 9 mutations" | — | — |
| **L79** | **BUG #1: "This model does not support assistant message prefill"** | 0 | 400 error |
| L81 | User: "try again" | — | — |
| L82 | API response after retry | **51.1k** | 54.1% (expected miss) |
| L84 | Assistant: "budget dropped from 54% to 29%, ~50k tokens cleared" | — | — |
| **L88** | **/context: 51k/200k (26%)** | — | **Archival WORKED** |

**Key finding**: Archival genuinely reduced API input from 112k → 51k. But the commit response triggered a 400 error because the rewritten payload ended with an assistant message.

---

### Phase 3: Status Check Loop (L90–L119) — CACHE CATASTROPHE

| Line | Event | API Input Tokens | Cache Hit % |
|------|-------|-----------------|-------------|
| L90 | User: "Check your context" | — | — |
| L93 | `aperture_context_status` #1 | 53.1k | 95.6% |
| L96 | Result: 27% budget, system block `e4916327` | — | — |
| L97 | `aperture_context_status` #2 | 53.3k | 99.2% |
| L100 | Result: 27% budget, system block `a5417238` (**ID changed**) | — | — |
| **L101** | `aperture_context_status` #3 | 53.3k | **51.9%** (25,639 tokens uncached) |
| L104 | Result: 27% budget, system block `992504a2` (**ID changed again**) | — | — |
| L105–L108 | Two parallel calls (status + preview) | 53.3k | 51.9% |
| L111 | Status result: system block `a13b421c` (**changed again**) | — | — |
| L114 | Preview result: 47 blocks, 27%, 12 suggestions (~296 tokens) | — | — |
| **L115** | **BUG #2: "final assistant content cannot end with trailing whitespace"** | 0 | 400 error |
| **L119** | **/context: 53k/200k (27%)**, Messages: 99.7k | — | — |

**Key findings**:
- System block IDs change on every request (content-addressed hashing sensitive to metadata drift)
- Cache dropped from 99.2% → 51.9% between consecutive calls (non-deterministic rewriting)
- `/context` Messages (99.7k) ≠ API input (53k) — CC's local bookkeeping ≠ post-rewrite payload

---

### Phase 4: Second Fill Attempt (L121–L155) — PERSISTENT ARCHIVAL LOOP

| Line | Event | API Input Tokens | Cache Hit % |
|------|-------|-----------------|-------------|
| L121 | User: "fill up context, hit 60%" | — | — |
| L125–L136 | Reads 4 files | 55.3k | 95.9% |
| L137–L151 | Reads 4 more files | 85.8k → 121.7k | 64–70% |
| L139 | Assistant: "Aperture's auto-archival keeps cleaning up the file reads" | — | — |
| L154 | Assistant: "persistent archival keeps re-archiving — budget snaps back to 11% each turn" | 121.7k | 70.3% |
| **L173** | **/context: 122k/200k (61%)** | — | — |

**Assessment**: Persistent archival re-applies committed archives on every request (by design for stateless clients). But this means newly-read file content gets immediately archived on next turn if it matches committed IDs. The model correctly identified this behavior.

---

### Phase 5: More Archival Attempts + Repeated Errors (L157–L217)

| Line | Event | API Input Tokens | Cache Hit % |
|------|-------|-----------------|-------------|
| L161 | `aperture_context_status` | 122.1k | 99.5% |
| L162 | `aperture_context_search` (parallel) | — | — |
| **L168** | **BUG #4: Search HTTP failure** (connection error to proxy) | — | — |
| **L169** | **BUG #1 repeat: "does not support assistant message prefill"** | 0 | 400 error |
| L175 | User: "try clearing some now" | — | — |
| L179 | `aperture_context_preview` | 124.3k | 98.0% |
| L182 | Result: 86 blocks, 58% budget, 38 suggestions (~2.7k) | — | — |
| **L183** | **BUG #1 repeat: "does not support assistant message prefill"** | 0 | 400 error |
| L185 | User: "try again" | — | — |
| L188 | Assistant: "9 blocks archived, 61% → 41%" | 124.4k | 99.7% |
| **L192** | **/context: 124k/200k (62%)** | — | No visible reduction in CC's view |
| L196 | **Extended thinking block** (1197 chars) | 126.5k | 98.1% |
| L201 | Preview: 97 blocks, 58% | — | — |
| L208 | Preview: 101 blocks, 59%, 48 suggestions | — | — |
| **L209** | **BUG #1 repeat: "does not support assistant message prefill"** | 0 | 400 error |
| **L213** | **/context: 127k/200k (64%)** | — | Context GREW despite archival |
| L216 | User: "hi" | — | — |
| **L217** | **BUG #3: "thinking blocks cannot be modified"** | 0 | 400 error |

---

## Bug Inventory

### BUG #1: Prefill Error — Payload Ends with Assistant Message (P0)

**Occurrences**: L79, L169, L183, L209 (4 times)
**Error**: `"This model does not support assistant message prefill"`
**Classification**: **ACTIVE BUG**

**Hypothesis**: After Aperture's cleanup strips MCP tool_result messages from conversation history, the corresponding assistant tool_use message is left orphaned at the end of the payload. Since there's no subsequent user message, the API rejects it.

**Investigation plan for Round 6**:
1. Examine the rewriter's cleanup path — does `cleanup_history()` strip tool_result but leave tool_use?
2. Check if `sanitize_anthropic_orphan_tool_uses()` runs AFTER cleanup
3. Check the ordering: cleanup → sanitize → forward. If sanitize runs before cleanup, orphans created by cleanup are never caught.
4. Reproduce with a minimal test: conversation ending with `[assistant: tool_use] [user: tool_result]`, run cleanup that strips the tool_result, verify the sanitizer catches the orphan.

---

### BUG #2: Trailing Whitespace Error (P1)

**Occurrences**: L115 (1 time)
**Error**: `"messages: final assistant content cannot end with trailing whitespace"`
**Classification**: **ACTIVE BUG**

**Hypothesis**: When the rewriter modifies assistant message content (e.g., replacing archived block content with stubs), it may leave trailing whitespace or newlines. The API enforces that the final assistant message has no trailing whitespace.

**Investigation plan for Round 6**:
1. Examine `apply_stubs_to_anthropic()` — does it trim content?
2. Check if text content blocks are being created with `\n\n` or trailing spaces
3. Look for any content concatenation in the rewriter that could introduce whitespace

---

### BUG #3: Thinking Block Modification (P1)

**Occurrences**: L217 (1 time, after L196 produced a thinking block)
**Error**: `"thinking or redacted_thinking blocks in the latest assistant message cannot be modified"`
**Classification**: **ACTIVE BUG**

**Hypothesis**: Aperture's rewriter processes ALL content blocks in assistant messages, including `thinking` type blocks. Anthropic requires thinking blocks to be passed through verbatim — they're signed and any modification (even whitespace) causes rejection.

**Investigation plan for Round 6**:
1. Check if the rewriter has a guard to skip `thinking` and `redacted_thinking` content blocks
2. Check if the parser/ingest modifies thinking block content (ANSI stripping? internal prompt filtering?)
3. Verify: does the rewriter touch thinking blocks during stub application, cleanup, or sanitization?

---

### BUG #4: Context Search HTTP Failure (P2)

**Occurrences**: L168 (1 time)
**Error**: `"HTTP request to proxy failed: error sending request for url (http://127.0.0.1:5400/_aperture/context/search)"`
**Classification**: **ACTIVE BUG** (intermittent)

**Hypothesis**: MCP binary's HTTP client timed out or the proxy was busy. Could be a transient issue or a panic in the search endpoint.

**Investigation plan**: Check proxy logs for panics around the same timestamp. Low priority.

---

### BUG #5: Cache Catastrophe on Multi-Tool Turns (P1)

**Occurrences**: L101 (25,639 tokens went uncached)
**Classification**: **ACTIVE BUG**

**Hypothesis**: Non-deterministic payload rewriting between consecutive requests within the same turn. System block IDs changed on every request (e4916327 → a5417238 → 992504a2 → a13b421c), suggesting the system prompt content or metadata differs slightly each ingest, producing different content-addressed hashes. This shifts the cache prefix.

**Investigation plan for Round 6**:
1. Check what makes system block content non-deterministic (timestamp? billing header? metadata?)
2. Check if block IDs are used in any payload position that affects cache prefix
3. This may be inherent to content-addressed hashing and not fixable without stable IDs for system blocks

---

### BUG #6: Block Count Inflation (P2)

**Occurrences**: 45 → 50 → 47 → 86 → 97 → 101 blocks over the session
**Classification**: **ACTIVE BUG**

**Hypothesis**: Every API request triggers a new ingest that creates new block IDs. MCP tool calls and results become blocks. The engine doesn't deduplicate blocks whose content is identical but whose metadata (turn_index, timestamp) differs.

**Investigation plan**: Examine ingest deduplication logic. May be expected for stateless clients (each ingest replaces session blocks entirely). If so, the count isn't "inflation" but normal session growth. The issue is that archival suggestions reference these transient blocks.

---

### BUG #7: Archival Doesn't Reduce CC's `/context` (P0, reframed)

**Occurrences**: Persistent — archival succeeded at API level (112k → 51k) but CC reported 99.7k messages
**Classification**: **EXPECTED BEHAVIOR** (reframed from P0)

**Explanation**: Claude Code's `/context` command estimates token usage from its local message store, which includes the full unmodified conversation. Aperture modifies the payload at the proxy level AFTER Claude Code has already counted its tokens. This is a fundamental architectural property of how CC works — it cannot know what the proxy did to its request.

**Implication**: Users will never see archival reflected in `/context` output. Only Aperture's own status tools show the post-rewrite budget. This should be documented as a known limitation and communicated to users via Aperture's UI or tool responses.

---

## P0 Fix Status from Previous Round

| Fix | Working? | Evidence |
|-----|----------|----------|
| CRITICAL-2 (MCP cleanup) | **Partially** | Block count still inflating (45→101). Some tool calls may not be stripped, or stubs are accumulating. |
| CRITICAL-1 (turn-aware projection + stubs) | **Yes** | First archival reduced API payload 112k → 51k. Stubs applied correctly. |
| MEDIUM-1 (session flip guard) | **Yes** | No Haiku session flips observed. Archive notifications are from persistent re-archival, not session flips. |

---

## Cache Performance Summary

| Phase | Avg Cache Hit % | Notes |
|-------|----------------|-------|
| Fill (L3–L51) | 70–99% | Normal, good cache |
| Archival tools (L58–L78) | 95–99% | Tool calls are small |
| Post-archival (L82) | 54% | Expected one-time miss |
| Status loop (L91–L97) | 95–99% | Recovered |
| **Status loop (L101–L115)** | **51.9%** | Non-deterministic rewriting |
| Fill round 2 (L122–L155) | 64–96% | Normal for new content |
| Late session (L158–L208) | 98–99% | Stable |

---

## Priority for Next Session (Round 6 Deep Dive)

1. **BUG #1 (P0)**: Prefill error — trace the cleanup → sanitize pipeline ordering
2. **BUG #3 (P1)**: Thinking block modification — find where thinking blocks get touched
3. **BUG #2 (P1)**: Trailing whitespace — find content construction path
4. **BUG #5 (P1)**: Cache catastrophe — investigate non-deterministic system block hashing
5. **BUG #7**: Document as known limitation (CC's /context ≠ Aperture's budget)

Bugs #1, #2, and #3 are likely all in the rewriter/cleanup pipeline and may share a common root cause: **the rewriter doesn't distinguish between content that can be modified and content that must be passed through verbatim**.

---

## Session Log Reference

```
File: ~/.claude/projects/-home-caden-projects-Aperture/db654aac-a155-4291-aae6-2cb1dfd20b31.jsonl
Size: 1.2MB
Lines: 217 JSONL entries
Key lines: L79 (first prefill error), L82 (archival success), L101 (cache catastrophe),
           L115 (whitespace error), L196 (thinking block), L217 (thinking modification error)
```
