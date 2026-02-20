# Cache Invalidation Root Cause Analysis

**Date:** 2026-02-14
**Status:** CONFIRMED — this is the primary cost driver
**Severity:** Product-viability threatening

---

## The Smoking Gun

Aperture's manifest injection into the system message **invalidates Anthropic's prompt cache prefix on every single request.** This is the dominant cost driver, dwarfing all other overhead sources by 1-2 orders of magnitude.

### Evidence Chain

1. **Anthropic's cache hierarchy is `tools → system → messages`.** Changes at any level invalidate that level and ALL subsequent levels. ([docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching))

2. **Cache keys are cumulative.** The hash for block N = hash(block_0, block_1, ..., block_N). If block 1 (system) changes, the hash for every subsequent block changes. ([docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching))

3. **Aperture injects a manifest into the system message that changes on EVERY request:**
   ```
   [Aperture: {pct}% budget | {blocks} blocks | {remaining} remaining]
   ```
   Budget percentage, block count, and remaining tokens change with every new message.
   Source: `engine/planner/manifest.rs`, `proxy/rewriter.rs:142-144`

4. **For Anthropic string system messages:** manifest is prepended via string concatenation.
   - Original: `"You are Claude Code..."`
   - Modified: `"[Aperture: 45% | 12 blocks | 110k remaining]\n\nYou are Claude Code..."`
   - This changes byte 0 of the system message → entire system + messages cache is invalidated.
   Source: `metacog/claude_mcp.rs` → `inject_manifest_anthropic` in `engine/planner/cleanup.rs`

5. **For Anthropic array system messages:** manifest is inserted as a new text block at index 0, pushing existing blocks (with their cache_control markers) downstream. The cumulative hash at every subsequent position changes.

6. **This happens on EVERY request, including streaming.** The manifest injection has NO streaming gate:
   ```rust
   // Tool injection is gated on !parsed.stream — but manifest is NOT:
   if manifest_eligible {
       runtime.inject_manifest(&mut json, &manifest_text);
   }
   ```

7. **Session `401b10df` confirms:** 46 requests, 5.34M cache_creation, **0 cache_read.** The cache NEVER hit because the system message changed on every request.

### What This Means Financially

For a 200k token conversation (Opus 4.6 pricing):

| Scenario | Tools (5k) | System+Messages (195k) | New Content (3k) | Total per request |
|---|---|---|---|---|
| **Without Aperture** | $0.003 cache_read | $0.098 cache_read | $0.019 cache_create | **$0.119** |
| **With Aperture (manifest)** | $0.003 cache_read | $1.219 cache_create | $0.019 cache_create | **$1.240** |
| **Overhead** | — | — | — | **$1.12 per request** |

Over 100 requests: **$112 pure cache invalidation overhead** (Opus 4.6)
Over 100 requests: **$331 pure cache invalidation overhead** (Opus 4.1)

For reference, the "Don't Break the Cache" paper ([arxiv](https://arxiv.org/html/2601.06007v1)) found that cache-aware agentic design saves 45-80% on costs. Aperture is currently doing the opposite — destroying the cache that Claude Code carefully maintains.

---

## Secondary Finding: Tool Injection Is NOT the Problem for Claude Code

For `ClaudeMcpRuntime` (the Claude Code path), `inject_tools()` is a **no-op**:

```rust
fn inject_tools(&self, _request_json: &mut Value) {
    // No-op — Claude Code discovers tools via MCP, not API request injection.
}
```

Tools are exposed through the MCP protocol handshake, not injected into the API request. This means the tools array in the API request is **UNCHANGED** by Aperture. The tools cache is stable.

This eliminates one of the initially hypothesized cost drivers. The tools are fine. **It's all the manifest.**

---

## Tertiary Finding: Block Archival Is Cache-Hostile But Recoverable

When Aperture removes a block from position K in the conversation:
- Everything before K: cache_read (unchanged) ✓
- Position K onward: cumulative hash changes → cache_create for all remaining tokens

One-time cost of archival:
```
cost = tokens_after_archival_point × (cache_create_rate - cache_read_rate)
     = 100k × ($6.25 - $0.50)/M = $0.575 per archival event
```

BUT: on the next request, if the same archival is applied, the modified prefix matches the previous request's cache → cache_read. The cost is one-time per new archival decision.

**Critical interaction:** This one-time-cost property ONLY works if the rest of the prefix is stable. If the manifest injection keeps changing the system message, the archival prefix never stabilizes and archival is always cache_create. **Fixing the manifest is prerequisite for archival to be cache-efficient.**

### 2026-02-19 Clarification: Persistent Archive Intent

For stateless full-history clients, previously archived blocks can reappear in each new raw request payload. Aperture must re-apply the same archive set each turn so the forwarded prefix remains shape-stable.

- This is expected and not inherently an additional per-turn cache penalty.
- With a stable archive set, transition cost is typically one-time when the shape first changes.
- Repeated high cache_create appears when archive sets oscillate (remove/reintroduce cycles), not when intent is consistently re-applied.

---

## What We Also Need to Consider

### Extended Thinking Cache Behavior
From the docs: "thinking changes (enabling/disabling or budget changes) will invalidate previously cached prompt prefixes with messages content." Claude Code uses extended thinking. If thinking parameters change between requests, that's another cache invalidation source independent of Aperture.

### 20-Block Lookback Window
The backward cache check only looks at 20 blocks before each explicit breakpoint. For conversations with 50+ blocks (common with tool-heavy Claude Code sessions), blocks more than 20 positions before the breakpoint fall out of the lookback window. This means even WITHOUT Aperture, very long conversations lose cache efficiency unless Claude Code sets multiple breakpoints.

### Breakpoint-Removal Edge Case (cache_control)
If archival removes a block carrying an explicit `cache_control` breakpoint marker, the breakpoint can disappear from that request shape and cause a larger transition miss. Aperture currently has no cache-control-aware archival guard, so this remains a known gap.

### Concurrent Requests (Subagents)
"A cache entry only becomes available after the first response begins." Concurrent subagent requests can't share caches because the first hasn't completed yet. This is an inherent cost of parallel tool use, not caused by Aperture.

### Breadcrumb Injection
Breadcrumbs also go through `inject_manifest()` into the system message. Same cache-busting behavior. Must be fixed alongside manifest.

### Context Tool Cleanup
`cleanup_history()` strips aperture_context_* tool_use/tool_result blocks from the conversation. This modifies message content in-place, changing the prefix. However, this is a correctness requirement (prevents orphan tool_result errors) and only costs one request of cache_create per cleanup.

---

## The Fix: Cache-Neutral Aperture

### Rule 1: NEVER modify the system message
Move manifest to the last user message (which is always new content → cache_create anyway) or remove it entirely. MCP tools provide the same information on demand.

### Rule 2: NEVER modify the tools array (already true for MCP path)
For non-MCP runtimes (Codex), make tool injection idempotent and stable. Consider MCP-only tool exposure.

### Rule 3: Make archival decisions stable
Once a block is archived, keep it archived. Don't flip-flop. This ensures the modified prefix stabilizes for cache hits.

### Rule 4: Append-only modifications
When possible, add content at the END of the conversation (after the cached prefix), never modify existing content. The only exception is archival (which is a one-time cache cost).

### Expected result after fix:
| Component | Before Fix | After Fix |
|---|---|---|
| Tools | cache_read (MCP, no change) | cache_read (no change) |
| System | cache_create EVERY request (manifest changes) | cache_read (never modified) |
| Messages prefix | cache_create EVERY request (system changed hash) | cache_read (stable prefix) |
| New content | cache_create (always new) | cache_create (always new) |
| Per-request overhead | **$1.12** (Opus 4.6) | **~$0** |

---

## Impact on Other Planned Work

### Delta Protocol (P5): Deprioritized further
Delta protocol reduces tool response payload size. But tool responses are in new content (always cache_create). Reducing them from 4000 tokens to 20 tokens saves ~$0.025 per call. Compared to $1.12 per request from manifest fix, this is negligible. Do it later.

### Economics Ledger (P1): Still essential
We need measurement to prove the fix works. But the fix is clear enough that we can implement it alongside the ledger.

### Schema Reduction (P3): Important for non-MCP paths only
For Claude Code (MCP path), tools aren't injected into the request. For Codex/OpenAI paths, tool injection IS happening and DOES change the tools array. Schema reduction matters for those paths.

### ROI Controller (P6): Still needed as safety net
Even after the fix, we should have auto-degrade as insurance.

---

## Sources
- [Anthropic Prompt Caching Docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- [Don't Break the Cache (arxiv)](https://arxiv.org/html/2601.06007v1)
- [Autocache Proxy (pattern reference)](https://github.com/montevive/autocache)
