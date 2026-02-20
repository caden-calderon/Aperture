# Deep Dive Diagnostics — Round 8

**Date**: 2026-02-19
**Sessions analyzed**: 2 manual test sessions (Fix A verification)
**Previous round**: Round 7 (infinite preview loop, cache death — Fix A implemented)

---

## Executive Summary

Fix A is **working**. MCP context tools are no longer stripped from conversation history, eliminating the infinite preview loop and cache death spiral from Round 7. However, two new bugs emerged — one critical, one model-specific:

1. **BUG R8-1 (P0)**: Thinking block corruption after archival causes unrecoverable API 400 errors. The rewriter modifies or shifts thinking/redacted_thinking blocks in the latest assistant message, which Anthropic rejects. This bricked the Sonnet session.

2. **BUG R8-2 (P2)**: Haiku cannot use `aperture_context_plan` — it confuses the tool's schema with `aperture_context_search`'s `query` parameter. 14 consecutive failures, zero tokens cleared.

3. **BUG R8-3 (P2)**: `aperture_context_search` returns no results for file content that exists in the engine's block store.

4. **BUG R8-4 (P2)**: Aperture vs `/context` token count divergence grows after archival (up to 45k gap after 2 archival passes), confusing users about actual savings.

**Cost**: Haiku session $0.28 (zero productive work), Sonnet session ~$1.80 (partial success before 400 bricked it).

---

## Session 1: Haiku ("sup haiku")

**File**: `4dfb71fe-9e05-457f-b19f-35f360ffd0c1.jsonl`
**Model**: `claude-haiku-4-5-20251001`
**Duration**: ~7 minutes (03:22:34 - 03:29:59 UTC)
**API calls**: 29
**Cost**: $0.28

### Timeline

| Phase | Time | Action | Result |
|-------|------|--------|--------|
| Setup | 03:22-25 | Switch to Haiku, fill context to 40% | Read 11 files, 4 status checks → 39% (77k/200k) |
| Verify | 03:25:58 | `/context` | 41% (82k/200k) — matches well |
| First clear | 03:26-27 | 1 preview + **8 failed plan calls** | Zero tokens freed. Haiku stuck. |
| Second clear | 03:28-29 | **6 more failed plan calls** | Zero tokens freed. |
| End | 03:29:59 | `/context` | 48% (96k/200k) — context GREW 7% from failed tool calls |

### BUG R8-2: Haiku Cannot Use `aperture_context_plan` Schema

Haiku correctly called `aperture_context_preview` (1 call, succeeded) and `aperture_context_status` (4 calls, all succeeded). But it fundamentally failed to understand `aperture_context_plan`'s parameter schema. **14 consecutive failures.**

The `aperture_context_plan` tool expects structured parameters:
```json
{ "archive": ["uuid1", "uuid2"], "control": {"op": "stage"} }
```

Haiku consistently sent everything as a `query` string (the parameter of `aperture_context_search`):

| Attempt | What Haiku Sent | Problem |
|---------|----------------|---------|
| 1 | `{"query": "stage archival goal=30000 tokens"}` | Wrong parameter name |
| 2 | `{"query": "archive #uuid1 #uuid2 ..."}` | UUIDs as hashtag-prefixed strings |
| 3 | `{"query": {"archive": [...], "control": {"op": "stage"}}}` | Correct structure INSIDE wrong param |
| 4-7 | Various `{"query": "..."}` strings | Tried natural language, key=value, etc. |
| 8 | `{"query": "archive #short-uuid ..."}` | Truncated UUIDs to 8 chars |
| 9-14 | More `{"query": "..."}` variations | Increasingly desperate |

**Root cause**: Haiku confused `aperture_context_plan` with `aperture_context_search`. The plan tool's `query` parameter was not in its schema, so the server silently ignored it and returned "No staged plan yet" — an error message that didn't help Haiku self-correct.

**The error message is the fixable part**: The server responds with "No staged plan yet. Call context_plan with actions (or control.op='stage') to begin strategic staging." This is too vague for smaller models. It should explicitly name the expected parameters and say `query` is not recognized.

### Cache Performance (Haiku)

| Metric | Value |
|--------|-------|
| Cache hit rate | **95.8%** overall |
| Cold-start (first call) | 0% (34k cache_create) |
| Subsequent calls | 75-99.7% |
| Total cost | $0.28 |
| Cost without caching | $1.83 |

Cache performance was excellent. No evidence of the old stripping-induced cache death. The `input_tokens` (uncached) field stayed at 3-69 tokens throughout — exactly what we'd expect with Fix A in place.

### Haiku Session Verdict

- Fix A: **WORKING** (tools available, preview/status succeed, no stripping)
- Infinite loop: **NOT PRESENT** (frustration loop, not preview loop)
- Cache: **HEALTHY** (95.8% hit rate)
- Archival: **FAILED** (tool schema confusion, not a proxy bug)
- New issue: Error messages need to be more helpful for smaller models

---

## Session 2: Sonnet ("sup sonnet")

**File**: `ef7012de-dc97-46be-ba52-f10bec4ce20b.jsonl`
**Model**: `claude-sonnet-4-6`
**Duration**: ~15 minutes (03:30:27 - 03:45:54 UTC)
**API calls**: 30
**Cost**: ~$1.80

### Timeline

| Phase | Time | Action | Result |
|-------|------|--------|--------|
| Setup | 03:30-31 | Fill context to 40% | 6 file reads → 39% (78k). `/context`: 40% (80k) |
| **1st clear (30k)** | 03:31-32 | preview → plan(stage) → plan(commit) | **SUCCESS**: 39% → 26% (53k). `/context`: 27% (54k) |
| Refill | 03:32-33 | Read 6 more files to 50% | 53% (105k). `/context`: 57% (115k) — 10k gap starts |
| **2nd clear (40k)** | 03:34-36 | preview → search(fail) → search(fail) → status → plan(stage) → plan(commit) | **PARTIAL**: Aperture says -41k. `/context`: 107k (only 8k freed) |
| Investigation | 03:36-44 | Sonnet investigates discrepancy. 3rd archival pass (15 more blocks) | Committed but then... |
| **API 400** | 03:44-45 | Thinking block modification error | **BRICKED**: 3 consecutive 400s, session unrecoverable |
| End | 03:45:54 | `/context`: 134k/200k (67%) | Context jumped 14% from investigation overhead |

### First Clear: SUCCESS

| Metric | Value |
|--------|-------|
| Target | 30k tokens |
| Blocks archived | 4 toolresult blocks (09af1009: 10.5k, 3f317a4b: 8.2k, 1a5e0646: 8.1k, 895ca30e: 4.2k) |
| Total archived | ~31k tokens |
| Before (Aperture / /context) | 39% (78k) / 40% (80k) |
| After (Aperture / /context) | 26% (53k) / 27% (54k) |
| Flow | preview → plan(stage) → plan(commit) |

Clean execution. Aperture and `/context` agree within 1-2%. The preview-stage-commit pattern worked exactly as designed. No loop. Model remembered its previous tool calls.

### Second Clear: PARTIAL SUCCESS (8k of 40k freed per /context)

| Metric | Value |
|--------|-------|
| Target | 40k tokens |
| Blocks archived | 7 blocks totaling ~41k (0f4d134e: 14k, 74a4da75: 6.8k, cf40de34: 6.6k, etc.) |
| Aperture projected | 38% budget, -41k net |
| Before /context | 115k/200k (57%) |
| After /context | 107k/200k (53%) — only 8k freed! |
| Aperture after | ~31% (62k) — claims 26k saved beyond /context's view |

The 33k gap between Aperture's claimed savings and /context's actual reduction is explained by:

1. **New tool call overhead**: The 2nd clear required 6 tool calls (98.5 second turn), adding ~19k tokens of new content to the conversation while clearing old content
2. **Aperture measures payload; /context measures local accumulation**: After archival, Aperture strips archived content from API payloads. But Claude Code's `/context` counts locally accumulated tokens. Aperture shows the API-level view (smaller), `/context` shows the client-level view (larger).
3. **Partial-turn stubs add overhead**: Archived blocks in partial turns get replaced with ~10 token stubs, not removed entirely

This is not a bug in archival itself — the savings ARE real at the API level — but the user-visible metric (`/context`) doesn't reflect them proportionally. This needs clearer communication.

### BUG R8-3: `aperture_context_search` Returns No Results

Sonnet called `aperture_context_search` twice during the second clear. Both returned "No matches." This forced Sonnet to fall back to `aperture_context_status` to find block IDs manually, which is less efficient and can produce truncated output.

**Investigation needed**: The `search_score()` function (metacog/tools.rs:575-607) searches `block.content`, `block.metadata.file_paths`, role, tool_name, and topic_keywords. Possible causes for no matches:
1. After archival, `block.content` may have been replaced with stub text in the engine store, so content-based matches fail
2. The search queries may not have matched any keywords in the remaining (non-archived) blocks
3. `session_blocks()` may exclude archived blocks, leaving fewer blocks to search

**Without the actual search queries from the JSONL, we cannot determine the exact cause.** This needs further investigation — either by extracting the queries from the JSONL or by adding search query logging.

### BUG R8-1: Thinking Block Corruption → Unrecoverable 400 (P0 — CRITICAL)

**Error**: `messages.7.content.3: 'thinking' or 'redacted_thinking' blocks in the latest assistant message cannot be modified. These blocks must remain as they were in the original response.`

**Impact**: Session bricked. 3 consecutive 400 errors (req IDs: `req_011CYJddnbjz1xHr9T2ECggE`, `req_011CYJdfDrobt98jkFwAQKhg`, `req_011CYJdfbcVqgEHy8TsuvF1M`). Every subsequent request also 400'd because the corrupted payload persisted.

**Root Cause Analysis** — Two contributing mechanisms in the rewriter pipeline:

#### Mechanism A: Partial-Turn Stub Index Mismatch After Full-Turn Removal

The rewriter pipeline in `apply_decisions_to_json()` (payload.rs:10-31) applies operations in this order:
1. `remove_anthropic_messages()` — removes entire messages by index (sorted reverse)
2. Content replacements
3. `apply_partial_turn_stubs_anthropic()` — replaces content blocks by `turn_index`

**The problem**: Both `remove_turns` and `partial_turn_stubs` use the ORIGINAL turn indices (computed by the applicator pre-removal). After step 1 removes messages, the array shifts. Step 3's `stub.turn_index - 1` now points to a DIFFERENT message than intended.

Example:
```
Original: messages[0..7] with turns [1..8]
Remove: turn 3, turn 5 → messages[2] and messages[4] removed
After removal: messages[0..5] (indices shifted)
Stub targets: turn_index=8 → messages[7-1=6], but only 6 messages exist now
  OR turn_index=7 → messages[6-1=5], which was originally messages[7]
  → messages[5] may be an assistant message with thinking blocks at content[3]
```

If the stub hits a thinking block (either directly or at the wrong content_index), the API rejects it.

#### Mechanism B: Message Merge After Turn Removal Creates Modified Assistant Messages

After payload rewriting, `sanitize_anthropic_message_structure()` (rewriter.rs:235) merges consecutive same-role messages. If turn removal creates two adjacent assistant messages (e.g., by removing a user message between them), the merge combines their content arrays — including thinking blocks from both messages.

The Anthropic API requires thinking blocks in the **latest** assistant message to be byte-identical to the original response. A merged assistant message containing thinking blocks from two different API responses violates this invariant.

**Test gap**: The existing test `test_structure_sanitizer_preserves_thinking_blocks_during_merge` (tests.rs:1001) verifies that thinking block *content* is preserved during merge. But the Anthropic API constraint is broader — the entire message structure must be unmodified, not just the thinking block fields.

#### Which Mechanism Caused This Specific Error?

The error points to `messages.7.content.3`. Given that the third archival pass archived 15 blocks (likely including full-turn removals), the most likely path is:

1. Full-turn removal shifts message indices
2. A partial-turn stub targeting `turn_index=X` now hits `messages[X-1-shift]`
3. That shifted message is the latest assistant message with a thinking block at `content[3]`
4. The stub replaces `content[3]` → API sees thinking block modified → 400

**Fix candidates** (analyzed, not yet implemented):

| Fix | Approach | Complexity |
|-----|----------|------------|
| **A1**: Adjust stub indices after turn removal | Subtract count of removed turns with index < stub.turn_index | Small |
| **A2**: Never stub/remove thinking blocks | `replace_content_block_with_stub()` should skip `"thinking"` and `"redacted_thinking"` types | Small |
| **A3**: Never modify the last assistant message | Exclude the last assistant turn from archival entirely | Medium |
| **A4**: Apply stubs before removal | Reorder pipeline: stubs first (indices correct), then removal | Small but needs careful review |

**Recommended**: A2 + A1 together. A2 is a safety guard (never touch thinking blocks). A1 fixes the index mismatch root cause. Both are small changes.

### Context Jump at End (+14%)

After the API errors, `/context` showed 134k/200k (67%), up from 107k (53%). This 27k/14-point increase was NOT random — it was Sonnet's investigation work between the 2nd clear and the 400 error:

| Turn | What Happened | Added Tokens |
|------|---------------|-------------|
| 26 | Investigation + preview | ~3k |
| 27 | More investigation | ~5k |
| 28 | Search + analysis | ~7k |
| 29 | Status check | ~6k |
| 30 | 3rd plan stage + commit | ~6k |
| **Total** | **5 investigation turns** | **~27k** |

The 3 failed API calls (400 errors) cost 0 tokens (rejected before processing) but the context was already at 134k by turn 30.

### Sonnet's Own Investigation Findings

Sonnet identified the Aperture-vs-/context discrepancy and analyzed it thoughtfully:

1. **Measurement difference**: Aperture measures the API payload (post-archival). `/context` measures local accumulated tokens (pre-archival). Both are "correct" for their domain.
2. **The 8k vs 41k paradox**: 41k was real API-level savings. New tool call overhead during the clear turn offset much of it from `/context`'s perspective.
3. **Persistent archival**: Sonnet correctly identified that blocks archived in the 1st clear were being re-archived each subsequent turn (working as designed).
4. **Missing block**: Sonnet found that `aperture_context_status` output was truncated, hiding the largest remaining block (applicator.rs, ~10k).

### Cache Performance (Sonnet)

| Turn Type | Cache Hit Rate | Notes |
|-----------|---------------|-------|
| Normal turns | 95-99% | Excellent |
| Post-1st-archival (turn 11) | **54.3%** | 23.6k cache_create, 28k cache_read |
| Post-2nd-archival (turn 25) | **26.4%** | 78k cache_create, 28k cache_read |
| Recovery (turns after archival) | 92-97% | Recovers within 1-2 turns |

**Key observation**: The 28,141 cache_read floor appears on BOTH post-archival turns. This is the stable prefix (system prompt + tool definitions + memory) that remains unchanged regardless of archival. Everything after the first archival point is a cache miss.

**This is expected behavior**, not a bug. Archival removes blocks from the message array, invalidating cumulative cache hashes from the removal point onward. The one-time cache_create cost ($0.38 for both archival events) is paid once; subsequent requests achieve 95%+ cache hit rates again.

**No evidence of the old ~90k uncached bug**: The `input_tokens` field (truly uncached tokens) stayed at 1-400 tokens throughout. This confirms Fix A eliminated the MCP tool stripping that caused cache death in Round 7.

| Metric | Sonnet Session |
|--------|---------------|
| Total cost | ~$1.80 |
| Cost without caching | ~$8.14 |
| Cache savings | $6.35 (77.9%) |
| Post-archival cache_create cost | $0.38 (21% of total) |

### BUG R8-4: Aperture vs /context Divergence Grows Over Time

| Timepoint | Aperture | /context | Gap |
|-----------|----------|----------|-----|
| Pre-clear 1 | 39% (78k) | 40% (80k) | +2k |
| Post-clear 1 | 26% (53k) | 27% (54k) | +1k |
| Pre-clear 2 | 53% (105k) | 57% (115k) | **+10k** |
| Post-clear 2 | ~31% (62k) | 53% (107k) | **+45k!** |

The gap grows with each archival pass because:
- Aperture's status reflects the post-archival payload (what the API sees)
- `/context` reflects Claude Code's local token accounting (what the user sees)
- Archived content is "invisible" to the API but still counted locally

This isn't a correctness bug but it's a **user experience problem**. The user asks "clear 40k" and `/context` only shows 8k freed. The real savings are happening but the user's primary feedback mechanism doesn't show them.

**Potential solutions** (design, not code):
1. Show both values in the UI: "API tokens" vs "local tokens"
2. After archival, show a notification: "Cleared 41k tokens from API payload. Your /context reading will catch up over subsequent turns."
3. Expose a "true API size" metric that `/context` can't see but Aperture can

---

## Sonnet's 5-Hour Usage Concern

The user noted usage seemed high between sessions — ~10% of the $100/month plan for relatively little work (Haiku reading ~10 files, Sonnet doing 15 minutes of testing). This warrants tracking:

| Session | Cost | Duration | Productive Work |
|---------|------|----------|-----------------|
| Haiku | $0.28 | 7 min | Zero (tool failures) |
| Sonnet | ~$1.80 | 15 min | 1 successful clear, 1 partial |
| **Total** | **~$2.08** | **22 min** | Modest |

At $100/month, 5-hour billing window, these two short sessions consumed ~2% of budget. The user's perceived 10% jump may include other sessions or billing granularity. Worth monitoring but not alarming — the cache savings ($7.88 total) are substantial.

**Note**: Sonnet's `/context` observation that "slash context is not always the true context" is **correct**. Claude Code's `/context` is an approximation based on local token counting. The actual API payload can differ due to:
- Tool injection (adds ~2-3k tokens not tracked locally)
- Content stripping (removes tokens not tracked locally)
- System prompt changes between API calls
- Compaction/compression applied by Claude Code itself

---

## Bug Summary

| # | Bug | Severity | Session | Root Cause | Fix |
|---|-----|----------|---------|------------|-----|
| **R8-1** | Thinking block corruption → unrecoverable 400 | **P0** | Sonnet | Partial-turn stub index mismatch after full-turn removal + missing thinking block guard | A2 (skip thinking blocks) + A1 (adjust indices) |
| **R8-2** | Haiku can't use aperture_context_plan | P2 | Haiku | Tool schema too complex for small models; error messages don't guide correction | Better error messages naming expected params |
| **R8-3** | aperture_context_search returns no results | P2 | Sonnet | Unknown — needs investigation (archived block content? search query mismatch?) | Investigate + add search logging |
| **R8-4** | Aperture vs /context divergence (up to 45k gap) | P2 | Sonnet | Architectural: Aperture measures API payload, /context measures local tokens | UX: surface both metrics or explain delta |
| R7-3 | Block count inflation from MCP tool interactions | P1 | Both | MCP tool blocks ingested into engine (Fix B from R7, not yet implemented) | Filter context tool blocks from ingest |
| R7-4 | Archival suggestions insufficient for large goals | P2 | — | Heuristics skip Recency zone (Fix C from R7, not yet implemented) | Expand heuristics to Recency zone |

---

## What IS Working (Confirmed by Both Sessions)

1. **Fix A (MCP tool preservation)**: MCP context tools stay in conversation history. No amnesia. No infinite loop.
2. **Cache health on normal turns**: 95-99%+ hit rate consistently. Zero evidence of the old stripping-induced cache death.
3. **First archival pass**: Clean preview → stage → commit flow. Sonnet achieved 30k clear exactly.
4. **Persistent archival**: Previously-archived blocks are re-stripped on subsequent requests automatically.
5. **Stub replacement**: Archived blocks replaced with compact stubs (~10-30 tokens) at the API level.
6. **Zero API 400 errors from Round 6 bugs**: The serde_json preserve_order fix and structural sanitization continue working.
7. **Post-archival cache recovery**: After the one-time cache_create hit, subsequent turns return to 95%+ cache hit.

---

## Priority and Fix Order

| Priority | Fix | Effort | Impact |
|----------|-----|--------|--------|
| **P0** | R8-1: Guard thinking blocks + fix stub index mismatch | Small | Prevents unrecoverable session bricking |
| **P1** | R7-3/Fix B: Filter MCP tool blocks from engine ingest | Small | Corrects block count, reduces engine bloat |
| **P2** | R8-2: Better error messages for plan tool | Small | Enables smaller models to use archival |
| **P2** | R8-3: Investigate search failures | Small | Better search → fewer tool calls → less overhead |
| **P2** | R8-4: Surface API vs local token divergence | Medium | Better user understanding of archival effectiveness |
| **P2** | R7-4/Fix C: Expand archival suggestions to Recency zone | Medium | Enables larger clear goals |

**Recommended implementation order**: R8-1 → R7-3 → R8-2 → R8-3 → R8-4 → R7-4

---

## Files Referenced

| File | Relevance |
|------|-----------|
| `src-tauri/src/proxy/rewriter/payload.rs:10-31` | `apply_decisions_to_json()` — pipeline order: remove → replace → stubs |
| `src-tauri/src/proxy/rewriter/payload.rs:208-241` | `apply_partial_turn_stubs_anthropic()` — uses original turn indices (bug) |
| `src-tauri/src/proxy/rewriter/payload.rs:316-348` | `replace_content_block_with_stub()` — no thinking block guard (bug) |
| `src-tauri/src/proxy/rewriter/payload.rs:37-61` | `remove_anthropic_messages()` — shifts array indices |
| `src-tauri/src/proxy/rewriter/sanitize.rs:197-252` | `sanitize_anthropic_message_structure()` — merges consecutive roles |
| `src-tauri/src/proxy/rewriter.rs:60-250` | Full rewriter pipeline order |
| `src-tauri/src/engine/planner/applicator.rs:101-297` | `apply_mutations()` — computes decisions with original indices |
| `src-tauri/src/metacog/tools.rs:352-405` | `context_search()` — search that returned no results |
| `src-tauri/src/metacog/tools.rs:575-607` | `search_score()` — content matching logic |
| `src-tauri/src/proxy/rewriter/tests.rs:1001-1025` | Test for thinking block preservation (insufficient) |

---

## Appendix A: Haiku Cache Pattern

```
Call  Input  CacheCreate  CacheRead   HitRate  Notes
0     10     34,369       0           0.0%     Cold start
1     10     232          34,369      99.3%
2     8      6,124        34,601      84.9%    File read
3     8      13,301       40,725      75.4%    File read
...
14    8      3,108        81,971      96.3%    Plan loop starts
15    3      283          85,079      99.7%
16    8      1,105        85,362      98.7%
...
28    69     1,263        95,418      98.7%    Plan loop ends
```

Cache performance remained excellent throughout despite 14 failed plan calls.

## Appendix B: Sonnet Cache Pattern

```
Turn  TotalInput  CacheRead  CacheCreate  HitRate  Notes
1     34,160      10,283     23,874       30.1%    Cold start
7     80,446      78,778     1,667        97.9%    Pre-clear 1
10    86,121      85,168     952          98.9%    Plan commit
11    51,872      28,141     23,591       54.3%    POST-ARCHIVAL 1 (expected)
12    54,451      51,732     2,580        95.0%    Recovery
18    114,919     110,787    3,993        96.4%    Pre-clear 2
24    127,897     125,659    2,099        98.3%    Plan stage
25    106,570     28,141     78,087       26.4%    POST-ARCHIVAL 2 (expected)
26    109,420     106,228    2,850        97.1%    Recovery
30    133,797     130,499    2,952        97.5%    3rd plan commit (pre-400)
```

Post-archival cache hits drop to 26-54% (one-time cost) then recover to 95%+ within 1-2 turns. The 28,141 cache_read floor = stable system prefix.
