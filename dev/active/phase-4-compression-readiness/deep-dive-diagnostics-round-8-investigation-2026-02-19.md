# Deep Dive Diagnostics — Round 8 Investigation

**Date**: 2026-02-19
**Type**: Code investigation + JSONL forensics (follow-up to Round 8 initial analysis)
**Sessions re-analyzed**: Haiku `4dfb71fe`, Sonnet `ef7012de`
**Previous**: `deep-dive-diagnostics-round-8-2026-02-19.md` (initial analysis)

---

## Executive Summary

This investigation deepens the initial Round 8 analysis with code-level verification and JSONL forensics. Key findings:

1. **R8-1 is caused by THREE independent mechanisms**, not two. All three must be fixed.
2. **R8-3 (search returning no results)** has a clear root cause: full-query substring matching instead of per-term matching.
3. **R8-2 (Haiku plan confusion)** has a clear root cause: `#[serde(default)]` without `deny_unknown_fields` silently swallows wrong parameters.
4. **R8-4** confirmed architectural (not a bug).

---

## R8-1: Thinking Block Corruption — Full Root Cause Analysis

### The Error

```
messages.7.content.3: `thinking` or `redacted_thinking` blocks in the latest assistant message
cannot be modified. These blocks must remain as they were in the original response.
```

Three consecutive 400s with request IDs: `req_011CYJddnbjz1xHr9T2ECggE`, `req_011CYJdfDrobt98jkFwAQKhg`, `req_011CYJdfbcVqgEHy8TsuvF1M`. Session permanently bricked.

### The Three Contributing Mechanisms

#### Mechanism 1: Partial-Turn Stub Index Mismatch After Full-Turn Removal

**Status**: VERIFIED as primary trigger.

**Code path**: `proxy/rewriter/payload.rs:10-31`

```
apply_decisions_to_json():
  1. remove_anthropic_messages()  — removes messages, shifts array indices
  2. content replacements         — uses original turn indices (may hit wrong messages)
  3. partial_turn_stubs           — uses original turn indices (definitely hits wrong messages)
```

The applicator (`engine/planner/applicator.rs:101-297`) computes all `turn_index` values from the request blocks' `metadata.turn_index`. These are stable indices corresponding to the ORIGINAL message array positions. But after `remove_anthropic_messages()` removes entries, the array shifts. Step 3's `stub.turn_index - 1` now points to a different message.

**Concrete example from the Sonnet session**:

The session had ~61 messages. Persistent archival from rounds 1+2 re-applied full-turn removals (4 blocks from round 1, 7 from round 2 — some were full turns). After removal of N full turns, a stub targeting original `turn_index=X` hits `messages[X - 1 - N]` instead of `messages[X - 1]`.

If the shifted index lands on the latest assistant message, and the stub's `content_index` targets a thinking block position, the stub function is called on a thinking block. Even though the catch-all in `replace_content_block_with_stub()` is a no-op for thinking blocks (they have `"thinking"` key, not `"text"` or `"content"`), the API detects the structural anomaly from other modifications to the same or nearby messages.

**Turn index mapping** (verified in `proxy/parser/anthropic.rs:142-146`):
- `turn_index = 0` → system prompt
- `turn_index = N` → `messages[N - 1]`
- All content blocks in a single message share the same `turn_index`

#### Mechanism 2: Thinking Blocks Not Protected from Archival

**Status**: VERIFIED — new finding not in initial report.

**Code path**: `engine/planner/heuristics.rs:334-372` (`is_archival_candidate()`)

The archival candidate selection has role-based exclusions for:
- `Role::System` — indirectly via Primacy zone exclusion
- `Role::ToolUse` and `Role::ToolResult` — at low pressure levels

But `Role::Thinking` has **NO exclusion** at any level. Thinking blocks are treated identically to User/Assistant blocks for archival eligibility.

**Plan validation** (`engine/planner/validation.rs:20-150`) only checks block existence — no role-based filtering.

**Evidence from JSONL**: The 3rd archival plan (JSONL line 187) included 2 thinking blocks:
- `79b6e9d1` — 227 tokens, "Archival planning thoughts"
- `4d61c880` — 122 tokens, "Context clearing thoughts"

Both were partial-turn stubs (shared turn_index with non-archived blocks). Both passed through to `replace_content_block_with_stub()`, which is a no-op for thinking blocks. But they shouldn't have been archival candidates in the first place.

**Why this matters**: Even if Fix 3 (below) prevents the stub from reaching them, the model shouldn't be wasting tool calls archiving 122-227 token thinking blocks. And if a thinking block IS the only block at a turn, a full-turn removal would delete a message containing thinking blocks, potentially triggering Mechanism 3.

#### Mechanism 3: Message Merge Creates Invalid Thinking Block State

**Status**: VERIFIED — worse than initially suspected.

**Code path**: `proxy/rewriter/sanitize.rs:197-252` (`sanitize_anthropic_message_structure()`)

The sanitizer merges consecutive same-role messages by blindly concatenating content arrays (`merge_message_content()` at sanitize.rs:255-285). It has **ZERO thinking block awareness** — the word "thinking" does not appear anywhere in `sanitize.rs`.

**Scenario**:
```
Before removal:
  msg[4]: assistant [thinking_A(sig_1), text "analysis", tool_use "search"]
  msg[5]: user [tool_result "results"]           ← ARCHIVED (full-turn removal)
  msg[6]: assistant [thinking_B(sig_2), text "conclusion"]

After removal:
  msg[4]: assistant [thinking_A(sig_1), text "analysis", tool_use "search"]
  msg[5]: assistant [thinking_B(sig_2), text "conclusion"]

After orphan tool_use sanitization (msg[4]'s tool_use has no matching tool_result):
  msg[4]: assistant [thinking_A(sig_1), text "analysis"]
  msg[5]: assistant [thinking_B(sig_2), text "conclusion"]

After merge:
  msg[4]: assistant [thinking_A(sig_1), text "analysis", thinking_B(sig_2), text "conclusion"]
```

This violates Anthropic's API requirements:
1. **Thinking blocks must appear at the start** of assistant content — `thinking_B` appears after `text`
2. **Signatures are cryptographically bound** to their original API response — merging blocks from two responses produces invalid signatures
3. **The "latest assistant message" constraint** requires byte-identical thinking blocks — a merged message doesn't match any single API response

**Existing test gap**: `test_structure_sanitizer_preserves_thinking_blocks_during_merge` (tests.rs:1001) verifies that thinking block content survives the merge but does NOT verify:
- Thinking blocks remain at the start of the content array
- There is at most one set of thinking blocks per message
- The resulting structure is Anthropic-API-valid

### Which Mechanism Caused `messages.7.content.3`?

From JSONL forensics, the conversation had ~61 messages. After the 3rd archival's turn removals and the sanitizer's merges, the array collapsed to ~15 messages. `messages[7]` became the latest assistant message — a merged multi-step message with the structure:

```
content[0]  = thinking (3283 chars, sig_X)
content[1]  = text (792 chars)
content[2]  = tool_use (aperture_context_preview)
content[3]  = thinking (9600 chars, sig_Y)   ← ERROR HERE
content[4]  = text (422 chars)
content[5]  = tool_use (aperture_context_search)
content[6]  = thinking (9808 chars, sig_Z)
content[7]  = text (174 chars)
content[8]  = tool_use (aperture_context_status)
content[9]  = thinking (1884 chars, sig_W)
content[10] = text (138 chars)
content[11] = tool_use (plan stage)
content[12] = thinking (345 chars, sig_V)
content[13] = text (92 chars)
content[14] = tool_use (plan commit)
```

This is a multi-step agentic turn with interleaved thinking/text/tool_use — a SINGLE assistant response. The thinking blocks at [3], [6], [9], [12] are part of the same response.

**Most likely cause**: Mechanism 1 (index mismatch) + Mechanism 3 (merge). The rewriter removed earlier turns, shifted indices, and the sanitizer merged adjacent assistants, creating `messages[7]` as a composite message. The API detected that `content[3]` (a thinking block) was either modified from its original or that the message structure didn't match the original response.

### Fixes for R8-1 (ALL four needed — defense in depth)

| Fix | Location | What | Complexity |
|-----|----------|------|------------|
| **F1: Adjust stub indices** | `payload.rs:10-31` | After `remove_anthropic_messages()`, compute adjustment for each stub's turn_index | Small |
| **F2: Guard thinking blocks in stubs** | `payload.rs:316-348` | Add `"thinking" \| "redacted_thinking" => return` to match | Trivial |
| **F3: Never archive thinking blocks** | `heuristics.rs:334+` and `validation.rs:45+` | Exclude `Role::Thinking` from archival candidates + reject in validation | Small |
| **F4: Never merge messages with thinking blocks** | `sanitize.rs:213-236` | If either message contains thinking/redacted_thinking blocks, skip merge — insert synthetic user message instead | Small |

**Implementation order**: F2 → F3 → F1 → F4 (safety guards first, then root causes).

#### F1 Detail: Adjust Stub Indices

Two approaches:

**Option A (preferred): Apply stubs before removal**
```rust
// In apply_decisions_to_json():
apply_partial_turn_stubs_anthropic(json, &decisions.partial_turn_stubs);  // FIRST
remove_anthropic_messages(json, &decisions.remove_turns);                 // THEN
```
Pro: Simple reorder. Stubs use correct indices on the original array.
Con: Content replacements also use original indices — need to verify they work correctly with this reorder. Actually, content replacements run BETWEEN removal and stubs currently, so they already have the index mismatch bug for Anthropic format (they use `turn - 1` as array index). Reordering all three to: stubs → replacements → removal would fix both.

**Option B: Compute index offset**
```rust
// After removal, for each stub:
let removed_before = decisions.remove_turns.iter()
    .filter(|&&t| t < stub.turn_index)
    .count();
let adjusted_idx = (stub.turn_index - 1) as usize - removed_before;
```
Pro: Explicit, testable.
Con: More code, need to handle edge cases.

**Recommendation**: Option A (reorder to stubs → replacements → removal). Simpler and fixes both stubs and content replacements.

**Important**: This reorder is safe because:
- Stubs modify content WITHIN messages (don't add/remove messages)
- Content replacements modify content WITHIN messages (don't add/remove messages)
- Removal removes entire messages (doesn't modify remaining message content)
- Stubs and content replacements target different turns (applicator removes turns from content_replacements that overlap with remove_turns at line 282-284)

#### F4 Detail: Never Merge Messages with Thinking Blocks

```rust
// In sanitize_anthropic_message_structure(), before merge:
if prev_role == cur_role && prev_role == "assistant" {
    let has_thinking = |msg: &Value| {
        msg.get("content")
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().any(|b| {
                matches!(
                    b.get("type").and_then(|t| t.as_str()),
                    Some("thinking") | Some("redacted_thinking")
                )
            }))
            .unwrap_or(false)
    };
    if has_thinking(&messages[write]) || has_thinking(&messages[read]) {
        // Don't merge — insert synthetic user message instead
        // ... handle separately
    }
}
```

---

## R8-2: Haiku Plan Tool Schema Confusion — Root Cause

### Verified Root Cause

`PlanActions` struct uses `#[serde(default)]` on all fields without `#[serde(deny_unknown_fields)]`. This means:

1. Haiku sends `{"query": "stage archival goal=30000 tokens"}`
2. `normalize_plan_arguments()` doesn't recognize `query` — passes through unchanged
3. `serde_json::from_value::<PlanActions>()` silently ignores unknown `query` field
4. All action fields default to empty (Vec::new(), HashMap::new())
5. `has_action_payload()` returns false
6. `parse_plan_control_op()` defaults to `"preview"` (no actions, no explicit op)
7. Response: "No staged plan yet. Call context_plan with actions (or control.op='stage') to begin strategic staging."

Haiku receives this response 14 times and never learns the actual parameter names because the error message doesn't list them.

### Fix

**F5: Detect unknown top-level keys and return helpful error**

In `normalize_plan_arguments()` or `context_plan()`, check for unrecognized keys and return a specific error:

```
"Unknown parameter 'query'. aperture_context_plan expects these parameters:
  - archive: [block_id, ...] — remove blocks from context
  - pin: [block_id, ...] — pin blocks to prevent archival
  - unpin: [block_id, ...] — unpin blocks
  - expand: [block_id, ...] — restore archived blocks
  - shift_to: {block_id: zone, ...} — move blocks between zones
  - compress: {block_id: summary, ...} — replace with summary
  - control: {op: 'stage'|'append'|'preview'|'commit'|'discard'}
Call aperture_context_preview first to get block IDs."
```

Alternatively: Add `#[serde(deny_unknown_fields)]` to `PlanActions`, which would cause the serde error to explicitly name the unknown field. However, this is more brittle — models may send benign extra fields. The explicit detection approach is more user-friendly.

---

## R8-3: Search Returns No Results — Root Cause

### Verified Root Cause

`search_score()` at `metacog/tools.rs:575-607` uses the **entire query as a single substring**:

```rust
let content_matches = content_lower.matches(query_lower).count() as u32;
```

Sonnet's queries from the JSONL:
1. `"heuristics applicator storage session ingest handler file read"` (63 chars)
2. `"apply_heuristics SqliteStorage ingest"` (37 chars)
3. `"PartialTurnStub RewriteDecisions apply_mutations"` (49 chars)

No block content contains these exact multi-word strings as contiguous substrings. The search is fundamentally broken for multi-term queries — which is the most common search pattern.

The same issue affects `file_paths` matching (`path.to_lowercase().contains(query_lower)`) and `topic_keywords` matching (`kw.to_lowercase().contains(query_lower)`).

### Fix

**F6: Tokenize queries and score per-term**

```rust
fn search_score(block: &Block, query_lower: &str) -> u32 {
    let mut score = 0u32;
    let content_lower = block.content.to_lowercase();
    let role_str = format!("{:?}", block.role).to_lowercase();

    // Split query into individual terms
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    if terms.is_empty() {
        return 0;
    }

    for term in &terms {
        // Content matches (capped per term)
        let content_matches = content_lower.matches(term).count() as u32;
        score += content_matches.min(3) * 2;

        // File path matches
        for path in &block.metadata.file_paths {
            if path.to_lowercase().contains(term) {
                score += 3;
            }
        }

        // Role match
        if role_str.contains(term) {
            score += 2;
        }

        // Tool name match
        if let Some(ref tool) = block.metadata.tool_name {
            if tool.to_lowercase().contains(term) {
                score += 3;
            }
        }

        // Topic keyword match
        for kw in &block.topic_keywords {
            if kw.to_lowercase().contains(term) {
                score += 2;
            }
        }
    }

    // Bonus for matching multiple terms (queries with more matches are more relevant)
    let matching_terms = terms.iter()
        .filter(|t| content_lower.contains(**t))
        .count() as u32;
    if matching_terms > 1 {
        score += matching_terms * 2;
    }

    score
}
```

Also need to update `extract_search_snippet()` to find snippets around individual terms.

---

## R8-4: Aperture vs /context Divergence — Confirmed Architectural

### Verified Explanation

| Metric | What It Measures | Source |
|--------|-----------------|--------|
| Aperture status | Post-archival API payload size | `engine.session_budget_status()` |
| `/context` | Claude Code's local token accumulation | Claude Code internal |

After archival:
- Aperture removes archived blocks from its budget calculation → reports smaller size
- Claude Code has no knowledge of Aperture's archival → continues counting all tokens
- The gap grows with each archival pass: +1k after round 1, +10k after round 2, +45k after round 3

No code fix needed for correctness. UX improvement: After successful archival commit, include:
```
Note: /context readings will not reflect these API-level savings.
Aperture's actual API payload: Xk tokens (Y% of budget).
```

---

## Verification: What's Working (Reconfirmed)

1. **Fix A from Round 7**: MCP context tools preserved in conversation — no amnesia, no infinite loop
2. **Cache health**: 95-99% hit rate on normal turns, expected one-time miss post-archival
3. **Archival execution**: First clear in Sonnet session was clean (preview → stage → commit, -31k)
4. **Persistent archival**: Re-sent blocks correctly re-archived
5. **Round 6 fixes**: Zero API 400s from structural/ordering issues (only from thinking block corruption)

---

## Complete Fix Priority Table

| Fix | Bug | Severity | Complexity | Description |
|-----|-----|----------|------------|-------------|
| **F2** | R8-1 | P0 | Trivial | Guard thinking blocks in `replace_content_block_with_stub()` |
| **F3** | R8-1 | P0 | Small | Exclude `Role::Thinking` from archival candidates + validation |
| **F1** | R8-1 | P0 | Small | Reorder pipeline: stubs → replacements → removal |
| **F4** | R8-1 | P0 | Small | Never merge assistant messages containing thinking blocks |
| **F6** | R8-3 | P2 | Small | Tokenize search queries per-term |
| **F5** | R8-2 | P2 | Small | Detect unknown plan params, list expected ones |
| — | R8-4 | P3 | Trivial | Add divergence note to archival commit output |
| **Fix B** | R7-3 | P1 | Small | Filter context tool blocks from engine ingest |
| **Fix C** | R7-4 | P2 | Medium | Expand archival suggestions to Recency zone |

**Implementation order**: F2 → F3 → F1 → F4 → Fix B → F6 → F5 → R8-4 → Fix C

---

## Files Referenced

| File | Lines | Role in Investigation |
|------|-------|----------------------|
| `proxy/rewriter/payload.rs` | 10-31, 37-61, 208-241, 316-348 | Stub application pipeline, thinking block catch-all |
| `proxy/rewriter/sanitize.rs` | 197-252, 255-285 | Message merge logic (no thinking awareness) |
| `proxy/rewriter.rs` | 170-242 | Full pipeline order |
| `proxy/rewriter/tests.rs` | 1001-1025 | Insufficient thinking block merge test |
| `engine/planner/applicator.rs` | 101-297 | Turn_index computation, partial-turn stub generation |
| `engine/planner/heuristics.rs` | 334-372 | `is_archival_candidate()` — no Thinking exclusion |
| `engine/planner/validation.rs` | 20-150 | `validate_plan()` — no role check |
| `metacog/tools.rs` | 575-607 | `search_score()` — full-query substring match bug |
| `metacog/tools/plan.rs` | 89-166, 169-245 | `normalize_plan_arguments()`, `context_plan()` |
| `proxy/parser/anthropic.rs` | 142-146 | turn_index assignment: `(i + 1) as u32` |
| Sonnet JSONL | lines 187, 193, 197, 201, 204 | 3rd archival plan, error messages |
| Sonnet JSONL | lines 130, 136, 175 | Search queries returning no results |

---

## Appendix: JSONL Evidence

### 3rd Archival Plan (15 blocks, 2 thinking)

From JSONL line 187 (plan stage):
- 13 non-thinking blocks: tool_result and user blocks (3k-10k tokens each)
- 2 thinking blocks: `79b6e9d1` (227 tok) and `4d61c880` (122 tok)
- All 15 were partial-turn stubs (shared turns with non-archived blocks)
- Projected: -34k tokens, 39% budget

### Search Queries (All Zero Matches)

| JSONL Line | Query | Result |
|------------|-------|--------|
| 130 | `"heuristics applicator storage session ingest handler file read"` | No matches |
| 136 | `"apply_heuristics SqliteStorage ingest"` | No matches |
| 175 | `"PartialTurnStub RewriteDecisions apply_mutations"` | No matches |

### Haiku Plan Failures (14 consecutive)

All used `{"query": "..."}` parameter instead of proper `archive`/`control` params. Serde silently ignored `query`, defaulted to "preview" op, returned unhelpful "No staged plan yet" message.
