# Deep-Dive Diagnostics Round 6 — Root Cause Analysis

**Date**: 2026-02-19
**Session**: `db654aac-a155-4291-aae6-2cb1dfd20b31` (Round 5 manual test)
**Investigator**: Claude Opus 4.6 (fresh context)
**Status**: Root causes confirmed for all 4 bugs. No code changes yet.

---

## BUG #1 (P0): "This model does not support assistant message prefill"

### Confirmed Root Cause

**`strip_anthropic_context_tools()` removes user messages that were entirely `tool_result` blocks, leaving consecutive assistant messages and/or the payload ending with an assistant message.**

The orphan sanitizers (`sanitize_anthropic_orphan_tool_uses`, `sanitize_anthropic_orphan_tool_results`) run AFTER cleanup (correct order) but do NOT address the structural violation because:
1. The orphaned blocks (tool_uses) have ALREADY been stripped by cleanup — the sanitizer has nothing to strip.
2. The real problem is that entire USER messages are removed (they had only tool_result content for context tools), leaving the conversation structurally invalid.
3. The orphan sanitizer for tool_uses explicitly SKIPS the last assistant message (line 145-147 of `sanitize.rs`: "Last assistant message - pending turn, don't strip").

### Code Path Trace

```
rewrite_request() [rewriter.rs:46]
  → runtime.cleanup_history(&mut json) [rewriter.rs:173]
    → strip_anthropic_context_tools(messages) [cleanup.rs:88]
      → Pass 1: collect context tool_use IDs from assistant messages [cleanup.rs:96-116]
      → Pass 2: strip tool_use from assistant, tool_result from user [cleanup.rs:127-170]
      → Pass 3: remove messages with empty content arrays [cleanup.rs:173-179]
        ← USER messages that had ONLY context tool_results are now empty → REMOVED
        ← ASSISTANT messages that had text + context tool_use keep their text → SURVIVE
  → sanitize_anthropic_orphan_tool_uses(&mut json) [rewriter.rs:217]
    → For last assistant message: SKIP (line 145-147: "pending turn, don't strip") ← NO HELP
    → For mid-conversation assistants: tool_use blocks already stripped by cleanup → NOTHING TO DO
  → serde_json::to_vec(&json) [rewriter.rs:230]
    ← Payload ends with assistant message → Anthropic rejects as prefill
```

### Minimal Reproduction Scenario

**Input** (Anthropic messages array):
```json
[
  {"role": "user", "content": [{"type": "text", "text": "clear context"}]},
  {"role": "assistant", "content": [
    {"type": "text", "text": "Let me check..."},
    {"type": "tool_use", "id": "t1", "name": "mcp__aperture__aperture_context_preview", "input": {}}
  ]},
  {"role": "user", "content": [
    {"type": "tool_result", "tool_use_id": "t1", "content": "Preview data..."}
  ]},
  {"role": "assistant", "content": [
    {"type": "text", "text": "I'll archive..."},
    {"type": "tool_use", "id": "t2", "name": "mcp__aperture__aperture_context_plan", "input": {}}
  ]},
  {"role": "user", "content": [
    {"type": "tool_result", "tool_use_id": "t2", "content": "Committed"}
  ]}
]
```

**After cleanup** (all context tools stripped, empty user messages removed):
```json
[
  {"role": "user", "content": [{"type": "text", "text": "clear context"}]},
  {"role": "assistant", "content": [{"type": "text", "text": "Let me check..."}]},
  {"role": "assistant", "content": [{"type": "text", "text": "I'll archive..."}]}
]
```

**Result**: Two consecutive assistant messages, payload ends with assistant → 400 "prefill" error.

### Proposed Fix

Add a **post-cleanup structural sanitizer** for Anthropic messages that:

1. **Merges consecutive same-role messages**: If cleanup produces `[assistant, assistant]`, merge their content arrays into a single assistant message.
2. **Ensures the last message is role=user**: If the payload ends with an assistant message, either:
   - (a) Remove trailing assistant messages that contain ONLY context-tool-cleanup residue (text like "Let me check..."), OR
   - (b) Append a minimal synthetic user message: `{"role": "user", "content": [{"type": "text", "text": "Continue."}]}`

Option (a) is cleaner but loses model output. Option (b) is safer — it preserves model text and lets the conversation continue naturally. **Recommend option (b)** since the text content may be meaningful to the user.

This sanitizer should run AFTER both cleanup and orphan sanitization, at `rewriter.rs:224` (before re-serialization).

---

## BUG #3 (P1): "thinking or redacted_thinking blocks cannot be modified"

### Confirmed Root Cause

**`serde_json` without `preserve_order` feature uses `BTreeMap` for JSON objects, which sorts keys alphabetically during round-trip. This changes the key ordering in thinking blocks, which Anthropic's integrity verification detects as a modification.**

Evidence from Cargo.toml:20 — `serde_json = "1"` with NO features. No `preserve_order`.

Evidence from JSONL L217 — error at `messages.9.content.1`: specific content block index pinpointing a thinking/redacted_thinking block.

### Code Path Trace

```
rewrite_request() [rewriter.rs:46]
  → ANY modification triggers re-serialization path
  → serde_json::from_slice(body) [rewriter.rs:160]
    ← JSON parsed into Value tree
    ← All objects use BTreeMap (alphabetical key order)
    ← Original thinking block: {"type": "thinking", "thinking": "...", "signature": "..."}
    ← After BTreeMap: keys stored as {"signature", "thinking", "type"} (alphabetical)
  → [various modifications to OTHER parts of the JSON]
  → serde_json::to_vec(&json) [rewriter.rs:230]
    ← Thinking block serialized as: {"signature":"...","thinking":"...","type":"thinking"}
    ← Original key order was: {"type":"thinking","thinking":"...","signature":"..."}
    ← KEY ORDER CHANGED → Anthropic detects as "modified"
```

### Why This Affects ALL Rewrites

The issue is NOT that any code explicitly modifies thinking blocks. No code path targets thinking blocks:
- `replace_content_block_with_stub()` [payload.rs:316-349]: default `_` case doesn't match thinking blocks (no `text` or `content` key)
- `replace_message_content()` [payload.rs:162-199]: only modifies `type=text` blocks
- `strip_anthropic_context_tools()` [cleanup.rs]: only strips `tool_use`/`tool_result`
- Orphan sanitizers [sanitize.rs]: only examine `tool_use`/`tool_result`

The problem is that **ANY rewrite** (cleanup, stubs, trailing context, tool injection) triggers the full `from_slice → to_vec` round-trip at `rewriter.rs:160+230`. This re-serializes the ENTIRE JSON body, including thinking blocks that were never targeted by any modification. The `BTreeMap` key reordering silently changes their JSON representation.

### Minimal Reproduction Scenario

**Input** (request body with thinking block in history):
```json
{
  "messages": [
    {"role": "assistant", "content": [
      {"type": "thinking", "thinking": "Deep analysis...", "signature": "ErUBCk..."},
      {"type": "text", "text": "My response"}
    ]},
    {"role": "user", "content": "hi"}
  ]
}
```

**After `serde_json::from_slice` → `serde_json::to_vec`** (with BTreeMap):
```json
{
  "messages": [
    {"content": [
      {"signature": "ErUBCk...", "thinking": "Deep analysis...", "type": "thinking"},
      {"text": "My response", "type": "text"}
    ], "role": "assistant"},
    {"content": "hi", "role": "user"}
  ]
}
```

**Result**: Every JSON object has alphabetically-sorted keys. The thinking block's key order changed from `{type, thinking, signature}` to `{signature, thinking, type}`. Anthropic rejects this.

### Proposed Fix

**Option A (recommended): Enable `preserve_order` feature on `serde_json`.**

Change `Cargo.toml:20` from:
```toml
serde_json = "1"
```
to:
```toml
serde_json = { version = "1", features = ["preserve_order"] }
```

This switches `serde_json::Map` from `BTreeMap` to `IndexMap`, which preserves insertion order during deserialization. Round-tripping would preserve the original key order in ALL JSON objects, not just thinking blocks.

**Option B (surgical): Preserve original thinking block bytes.**

Before parsing, extract raw JSON bytes for each thinking/redacted_thinking block. After re-serialization, splice the original bytes back in. This is more complex and error-prone than Option A.

**Recommend Option A.** It's a one-line dependency change with no code modifications needed. The `IndexMap` has nearly identical performance to `BTreeMap` for this use case. It also prevents any other future issues caused by key reordering in other JSON structures (cache_control, tool definitions, etc.).

---

## BUG #2 (P1): "final assistant content cannot end with trailing whitespace"

### Confirmed Root Cause

**This is a consequence of BUG #1, not an independent bug.** When cleanup leaves the payload ending with an assistant message, Anthropic applies "prefill" validation rules to that message. One such rule: prefill content cannot end with trailing whitespace.

The trailing whitespace was NOT introduced by Aperture — it was in the original model output (natural language text often ends with whitespace/newlines). The whitespace only becomes invalid when the assistant message is repositioned as the final message (= prefill).

### Evidence

- Single occurrence at L115
- Same pattern as BUG #1: after tool cleanup, payload ends with assistant message
- Anthropic validated the prefill content (not the "no prefill" error) and found trailing whitespace
- The different error message ("trailing whitespace" vs "does not support prefill") suggests Anthropic's validation order varies or the model partially supports prefill under certain conditions

### Code Path Trace

Same as BUG #1, but Anthropic reaches a different validation check first:
```
cleanup strips context tools → user messages removed → assistant at end
→ Anthropic checks: is last message assistant? YES → validate as prefill
→ Anthropic checks: does prefill content end with whitespace? YES → 400 error
```

### Proposed Fix

**Fixing BUG #1 fixes BUG #2 automatically.** The post-cleanup structural sanitizer (from BUG #1's fix) ensures the payload never ends with an assistant message, so prefill validation never triggers.

As defense-in-depth: when merging consecutive assistant messages (BUG #1 fix step 1), trim trailing whitespace from the merged text content. This protects against edge cases where an assistant message legitimately ends up as the last message.

---

## BUG #5 (P1): Cache Catastrophe — Non-deterministic System Block IDs

### Confirmed Root Cause

**The system prompt content starts with `x-anthropic-billing-header:` values that change per request. The block ID generator uses `content_fingerprint()` (hash of first 200 chars) which includes this dynamic header, producing different block IDs on each request.**

### Evidence (from JSONL)

| Line | System Block ID | Content Preview |
|------|----------------|-----------------|
| L96  | `e4916327-3ade-5c24-...` | `x-anthropic-billing-header: cc_version=2.1.47.b96; cc_entryp...` |
| L100 | `a5417238-0807-5085-...` | `x-anthropic-billing-header: ...` (different value) |
| L104 | `992504a2-243f-55d4-...` | `x-anthropic-billing-header: ...` (different again) |
| L111 | `a13b421c-...` | `x-anthropic-billing-header: ...` (different again) |

Four different system block IDs in four consecutive requests. The content preview shows the billing header at the START of the system content, which is within the first 200 characters used by `content_fingerprint()`.

### Code Path Trace

```
parse_anthropic_request() [anthropic.rs:86]
  → Parse system field [anthropic.rs:100-127]
    → System content = "x-anthropic-billing-header: cc_version=2.1.47.b96; cc_entryp...\nYou are Claude Code..."
    ← Billing header is the FIRST content (within first 200 chars)
  → content_fingerprint(&system_content) [mod.rs:245-247]
    → short_hash(first 200 chars) [mod.rs:234-238]
    ← Hash includes billing header → different hash each request
  → stable_block_id(System, "anthropic", fingerprint, "anthropic:system:0") [mod.rs:153-170]
    ← Different fingerprint → different block ID

The engine KNOWS about billing headers:
  → normalize_regression_content() [ingest.rs:257-266]
    → Filters out "x-anthropic-billing-header:" lines for regression comparison
    ← But this filter is NOT used in content_fingerprint()!
```

### Cascade Effects

The non-deterministic system block ID causes:

1. **Engine instability**: Each ingest sees a "new" system block (different ID). The old block is removed and replaced. This causes the session's block list to churn even though the semantic content is identical.

2. **Planner state drift**: The planner's persistent archival queue stores block IDs. If the system block ID changes, the engine's view of the session changes. While the system block itself is unlikely to be an archival target (it's in Primacy), the block list churn can cause:
   - Different staleness rankings (all blocks get fresh `last_referenced_turn` on replacement)
   - Different block counts in budget calculations
   - Different heuristic signals

3. **Status/preview output changes**: The system block ID appears in `aperture_context_status` output. Different IDs each request mean the status text differs, which could influence model behavior.

4. **Cache invalidation (indirect)**: If the planner produces different mutations due to session churn, the payload modifications differ between requests, causing Anthropic's prefix cache to miss. The 25,639-token cache miss at L101 correlates with the system block ID change.

### Proposed Fix

**Normalize system content before fingerprinting by filtering dynamic headers.**

In `parser/mod.rs`, add a normalization step to `content_fingerprint()` or create a specialized `system_content_fingerprint()` that strips known-dynamic prefixes:

```
fn normalize_system_for_fingerprint(content: &str) -> String {
    content.lines()
        .filter(|line| !line.trim_start().to_ascii_lowercase()
            .starts_with("x-anthropic-billing-header:"))
        .collect::<Vec<_>>()
        .join("\n")
}
```

Apply this normalization ONLY for system blocks (Role::System) before calling `content_fingerprint()`. This reuses the same filtering logic already in `normalize_regression_content()` (ingest.rs:257-266).

This ensures system block IDs are stable across requests with different billing headers, while still detecting genuine system prompt changes (e.g., CLAUDE.md edits, MCP tool changes).

---

## Priority Summary

| Bug | Root Cause | Fix Complexity | Dependency |
|-----|-----------|----------------|------------|
| **#1 (P0)** | Cleanup removes user messages → invalid structure | Medium — new post-cleanup sanitizer | None |
| **#3 (P1)** | serde_json key reordering during round-trip | Trivial — one-line Cargo.toml change | None |
| **#2 (P1)** | Consequence of #1 | None — fixed by #1 | Depends on #1 |
| **#5 (P1)** | Billing header in system content fingerprint | Low — normalize before fingerprint | None |

**Recommended fix order**: #3 (trivial, immediate) → #1 (fixes #2 too) → #5 (independent)

---

## Files to Modify (Preview)

| File | Bug | Change |
|------|-----|--------|
| `src-tauri/Cargo.toml` | #3 | Add `features = ["preserve_order"]` to serde_json |
| `src-tauri/src/proxy/rewriter/sanitize.rs` | #1 | Add `sanitize_anthropic_message_structure()` function |
| `src-tauri/src/proxy/rewriter.rs` | #1 | Call new sanitizer after orphan sanitizers |
| `src-tauri/src/proxy/parser/anthropic.rs` | #5 | Normalize system content before fingerprinting |
| `src-tauri/src/proxy/parser/mod.rs` | #5 | Add `normalize_system_for_fingerprint()` helper |
