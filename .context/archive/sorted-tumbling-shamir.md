# Plan: Post-Manual-Test Fixes (Phase 4, 2026-02-16)

## Context

Manual test of Prompts 1+2 ran after implementing token-proportional zones, tool block archival, token-based targets, ANSI stripping, and internal prompt filtering. Zones work. Archival pipeline verified correct. But 4 issues surfaced:

1. **Thinking blocks assigned to Primacy** — immune to archival, wasting budget
2. **Budget mismatch** — Aperture shows 67% (133k) while Claude Code shows 80% (160k)
3. **MCP/API errors** — 400 concurrency + proxy connection errors
4. **Archival effectiveness unclear** — model couldn't confirm context actually shrinking

Investigation results: Issue 3 is NOT an Aperture bug (400 is Anthropic API, proxy error is transient). Issue 4's pipeline is correct but was blocked by Issue 3. The real fixes needed are Issues 1 and 2.

---

## Fix A: Thinking Blocks Should Not Be in Primacy

**File**: `src-tauri/src/engine/zone.rs`

### Problem
Line 53: `if block.role == Role::System || block.role == Role::Thinking` assigns ALL thinking blocks to Primacy. This makes them immune to archival. In the manual test, 6 thinking blocks consumed 6.7k tokens in Primacy.

### Fix
Remove `Role::Thinking` from the Primacy special-case. Let thinking blocks follow the same token-proportional rules as User/Assistant blocks.

```rust
// zone.rs assign_zones(), Pass 1 — change this:
if block.role == Role::System || block.role == Role::Thinking {
    block.zone = Zone::BuiltIn(BuiltInZone::Primacy);
    continue;
}

// TO this:
if block.role == Role::System {
    block.zone = Zone::BuiltIn(BuiltInZone::Primacy);
    continue;
}
```

### Tests
- Update `test_thinking_goes_to_primacy` → `test_thinking_follows_token_proportional_rules`
- Verify: with <10k non-system tokens, thinking goes to Recency
- Verify: with >10k non-system tokens, old thinking goes to Middle (archivable)

---

## Fix B: Budget Overhead Tracking

**Problem**: Aperture counts only conversation message blocks. Claude Code's actual context includes:
- System tools: ~17k tokens (built-in tool schemas like Read, Bash, Grep, etc.)
- MCP tools: ~3.7k tokens (aperture, svelte, crates, context7 schemas)
- Compact buffer: ~3k tokens
- Skills: ~61 tokens
- Total untracked overhead: ~24k tokens

This causes heuristics to fire ~12 percentage points late (67% when really 80%).

### Approach: Extract tool token overhead from request JSON

During `ingest()`, the raw request body is available. We can count the `tools` array tokens.

**Files to modify**:

1. **`src-tauri/src/proxy/handler.rs`** — Already has `body_bytes`. After parsing, estimate tool overhead from the `tools` array in the JSON, pass to engine.

2. **`src-tauri/src/proxy/parser.rs`** — Add `overhead_tokens: u32` field to `ParsedRequest`. In `parse_anthropic_request()` / `parse_openai_*()`, count tokens in the `tools` array using simple byte-length estimation (bytes / 4 is close enough for tool schemas).

3. **`src-tauri/src/engine/session.rs`** — Add `overhead_tokens: u32` to `Session`. Updated on each ingest.

4. **`src-tauri/src/engine/mod.rs`** — Accept `overhead_tokens` in `ingest()`, store in session. Add to `budget_status()` calculation.

5. **`src-tauri/src/engine/budget.rs`** — `budget_status()` takes optional overhead parameter, adds to used_tokens for utilization.

### Estimation approach
```rust
// In parser, after extracting tools array:
let overhead_tokens = if let Some(tools) = json.get("tools") {
    let tools_str = serde_json::to_string(tools).unwrap_or_default();
    (tools_str.len() as u32) / 4  // ~4 chars per token estimate
} else {
    0
};
```

This is intentionally approximate — 4 chars/token is close enough for JSON tool schemas, and it tracks changes automatically (adding/removing MCP tools changes the count).

### Tests
- Parse request with tools array → overhead_tokens > 0
- Parse request without tools → overhead_tokens = 0
- Budget utilization includes overhead → higher than messages-only

---

## Issue C: MCP/API Errors (NO FIX NEEDED)

### 400 "tool use concurrency"
This is **Anthropic's API** rejecting Claude Code's concurrent tool calls. Not Aperture.
- Evidence: `interceptor.rs` line 49 returns `None` for `RuntimeKind::ClaudeMcp` — interceptor never runs for Claude Code
- The MCP context tools go through the `aperture-mcp` binary → HTTP → `context_api.rs`, bypassing the interceptor entirely
- Anthropic sometimes returns 400 when Claude Code makes parallel tool calls

### Proxy connection error on `/_aperture/context/status`
Transient — likely the MCP binary's 30s timeout hitting during a slow response. The `RunawayGuard` mutex is held for microseconds (just `VecDeque::push_back`), not a contention issue.
- Could increase timeout in `aperture_mcp.rs` line 91 from 30s → 60s as a safety margin
- But this is low priority — the error is intermittent and fail-open

---

## Issue D: Archival Effectiveness (VERIFIED CORRECT, BLOCKED BY C)

### Pipeline trace (confirmed correct)
1. Heuristics generate `ContextMutation::Archive { block_id }`
2. `applicator.rs` converts to `RewriteDecisions { remove_turns }` — only removes turn if ALL blocks at that index are archived
3. `rewriter.rs` strips those turns from outgoing JSON before forwarding to LLM
4. `engine.archive_block_internal()` removes blocks from store
5. Next request: LLM re-sends full conversation, rewriter strips again

### Why it seemed ineffective
The 400 errors (Issue C) killed the conversation before archival could apply on the next request. Once those are avoided (they're transient), archival should work.

### Verification step
Re-run manual test. After archival fires, check proxy logs for:
- `"Removing N turns from payload"` — confirms JSON-level stripping
- Compare Aperture block count before/after archival
- Watch Claude Code's `/context` output for reduced token count

---

## Implementation Order

1. **Fix A** (thinking blocks) — 5 min, update zone.rs + tests
2. **Fix B** (budget overhead) — 30 min, parser + session + budget + ingest changes
3. **Optional**: Increase MCP binary timeout 30s → 60s in `aperture_mcp.rs:91`
4. `cargo test` + `cargo clippy` + `npx vitest run`
5. Re-run manual test Prompts 1+2, verify budget % matches Claude Code

## Files Modified

| File | Changes |
|------|---------|
| `src-tauri/src/engine/zone.rs` | Remove Thinking→Primacy, update test |
| `src-tauri/src/proxy/parser.rs` | Add overhead_tokens to ParsedRequest, extract from tools array |
| `src-tauri/src/engine/session.rs` | Add overhead_tokens field to Session |
| `src-tauri/src/engine/mod.rs` | Accept overhead in ingest(), use in budget_status() |
| `src-tauri/src/engine/budget.rs` | budget_status() accounts for overhead |
| `src-tauri/src/bin/aperture_mcp.rs` | (optional) increase timeout 30s→60s |

## Verification

1. All existing tests pass
2. New tests for thinking zone assignment + overhead token counting
3. Manual test: budget % within 5% of Claude Code's `/context` report
4. Manual test: archival reduces context (check before/after block counts)
