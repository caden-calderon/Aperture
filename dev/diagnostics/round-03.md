# Deep Dive Diagnostics Round 3 — Independent Forensic Review (2026-02-19)

## Scope
- Mode: Independent forensic review — confirming/refuting all round-2 findings with independent evidence chains, plus surfacing any additional defects discovered during deep tracing.
- No code fixes in this round.
- Repro artifacts:
  - Claude log: `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl`
  - Aperture DB: `~/.aperture/aperture.db`
- All assumptions validated against official Anthropic/Claude Code documentation (sources listed in Section 9).

---

## 1. Findings (Severity-Ordered)

### CRITICAL-1: Archive Mutations Do Not Reduce API Payload — Fundamental Architecture Gap

**Severity:** Critical (blocks ALL archival value)
**Proof status:** Proven (code + runtime + JSONL evidence)

This is the highest-severity finding and was **underweight in round-2 diagnostics**. The round-2 report correctly identified "projection can overstate savings" but framed it as a projection-math bug. **The real issue is deeper**: the `Archive` mutation has no mechanism to reduce the API payload unless every block in a turn is archived.

**Evidence chain:**
1. **Projection** (`engine/planner/validation.rs:236-270`): `estimate_token_delta()` sums `-block.tokens` for each archived block independently. 7 blocks → projects `-60,642` tokens.
2. **Applicator** (`engine/planner/applicator.rs:190-224`): Requires ALL blocks at a `turn_index` to be archived before adding to `remove_turns`. Partial-turn archives produce zero turn removals.
3. **Rewriter** (`proxy/rewriter.rs:148-156`): When `decisions.has_payload_changes()` is false (no turns to remove, no content replacements), returns `Ok(None)` — the original body is forwarded unchanged.
4. **Engine divergence** (`proxy/rewriter.rs:154`): Even when payload is unchanged, `apply_engine_updates()` still runs → `archive_block_internal()` removes blocks from engine store. Engine view and API view permanently diverge.
5. **JSONL proof** (repro log `66dd683a`, L76-L97):
   - Stage/commit reported "-61k tokens" projected savings
   - `/context` went 64% → 67% → 69% (monotonically increasing through and after archival)
   - `cache_read_input_tokens` on L91: 134,574 tokens — virtually identical to pre-archival 128,841 (difference = new conversation content, not archival)
6. **Replay test** (`engine/planner/tests.rs:602`): `token_delta = -60,642`, `remove_turns.is_empty()`, `has_payload_changes() == false`

**Why this is more than a projection bug:** There is no intermediate mechanism between "archive block" and "remove turn." No content-replacement fallback exists for partial-turn archives. The `Archive` mutation only produces `EngineUpdateKind::Archive` (engine-side) and a potential entry in `remove_turns` (payload-side, gated on full-turn coverage). For tool-heavy conversations where tool_result blocks share turns with non-archived blocks, **archival is fundamentally a no-op at the API level**.

---

### CRITICAL-2: Cleanup Misses All Namespaced MCP Tool Names

**Severity:** Critical (every Aperture context tool call persists in conversation history forever)
**Proof status:** Proven (code + JSONL + replay test)

**Evidence chain:**
1. **Matcher** (`metacog/runtime.rs:57-61`): `is_context_tool_name(name)` does `name.starts_with("aperture_context_")`. This is the single source of truth for all cleanup paths.
2. **Claude Code naming** (confirmed via official docs + JSONL evidence): MCP tools appear as `mcp__aperture__aperture_context_preview`, `mcp__aperture__aperture_context_plan`, etc. The `mcp__servername__` prefix is standard MCP protocol behavior, documented in Claude Code's MCP integration docs.
3. **JSONL evidence** (repro log `66dd683a`, L68/L73/L78): All three Aperture tool calls use `mcp__aperture__` prefixed names in the `tool_use` blocks.
4. **8+ match sites affected**: `cleanup.rs:108,145,159` (Anthropic), `cleanup.rs:221,254` (OpenAI chat), `cleanup.rs:327,348` (OpenAI responses), `interceptor/response.rs:59,115` (response interception).
5. **MCP server registration** (`mcp/server.rs:7-15`): Tools are registered with canonical names (`aperture_context_*`), but Claude Code adds the `mcp__aperture__` namespace on the wire.
6. **Replay test** (`cleanup.rs:583`): Confirms 0 tool uses stripped, 0 tool results stripped for namespaced names.

**Cascading impact:**
- Each unstripped context tool call/result persists indefinitely in the conversation (500-5000+ tokens per call)
- Over a multi-turn session with 10+ context tool invocations, this adds 5-50k tokens of stale context tool results to every subsequent API request
- The LLM sees old context snapshots from previous turns, potentially making decisions based on stale data
- Context tool overhead compounds the budget pressure that triggered the archival attempt in the first place

---

### HIGH-1: Active Session Flips on Auxiliary Ingests → False Archival Toasts

**Severity:** High (user-visible UI bugs, false archival notifications)
**Proof status:** Proven (code + mechanism test)

**Evidence chain:**
1. **Session creation** (`engine/session.rs:134`): `SessionStore::create()` unconditionally sets `active_id` to the new session. No concept of "primary" vs "auxiliary."
2. **Identity resolution** (`engine/mod.rs:863-887`): `ensure_session()` creates a new session for each unique `(provider, model, source, thread_id)` tuple. Haiku topic-classifier requests have a different model → different session → becomes active.
3. **UI refresh** (`+page.svelte:69-79`): `refreshEngineState()` calls `engine_get_blocks()` which returns `active_session_blocks()` — whatever session is currently active.
4. **Toast logic** (`context.svelte.ts:861-892`): `notifyBlockMutations()` compares old blocks (from primary session) with new blocks (from auxiliary session). All primary block IDs are missing → all reported as "archived."
5. **Guard ineffectiveness** (`engine/ingest.rs:197-246`): Regressive guards only protect a session from itself. They don't prevent session-flip side effects because auxiliary ingests create entirely new sessions with different IDs.
6. **Tests** (`engine/tests.rs:484`): Proves active flips to auxiliary session. (`context-budget.test.ts:169`): Proves session replacement triggers false archival toast.

---

### HIGH-2: Token Metric Divergence is Structural (3 Different Counting Domains)

**Severity:** High (user confusion, diagnostic noise)
**Proof status:** Proven (design-level, not a single-counter bug)

**Three distinct measurement domains:**
| Source | What it counts | Includes overhead? |
|--------|---------------|-------------------|
| Aperture engine budget | Sum of session block tokens + overhead_tokens | Yes (tools, protocol) |
| Aperture UI token bar | Sum of block tokens from active session blocks | No (block sum only) |
| Claude `/context` | All request components: system prompt, tools, memory, skills, messages + response reservation | Claude-internal categories not modeled by Aperture |

**Evidence:**
- Backend: `engine/mod.rs:184` includes `overhead_tokens`
- UI: `context.svelte.ts:248` calculates from block sum only (no overhead)
- JSONL: `/context` reports 64% while Aperture preview reports 62% at the same moment (L62 vs L71)
- Claude `/context` includes ~15-20k of system prompt + tools + memory that Aperture doesn't model as blocks

**Key distinction:** This is NOT a bug in the traditional sense — the three sources measure different things. But the user perceives it as "nothing works" when all three show different numbers.

---

### MEDIUM-1: Block ID Stability is Sound for Normal Conversation Growth, Fragile Under Edge Cases

**Severity:** Medium (persistent archival depends on it)
**Proof status:** Proven stable for the common case; edge cases identified

**Block ID generation** (`parser/mod.rs:153-170`):
```
stable_block_id(role, provider, content_fp, block_key)
→ hash(provider|Role|content_fingerprint|block_key)
```

Where:
- `content_fp` = hash of first 200 chars of content
- `block_key` = `"anthropic:{type}:{content_index}:{occurrence}"` for text, or `"anthropic:tool_use:{tool_use_id}:{occ}"` for tool blocks

**Stability analysis:**
- **Tool blocks (tool_use/tool_result)**: Highly stable. Keyed on `tool_use_id` which is immutable across turns.
- **Text blocks**: Stable as long as content prefix (200 chars) and occurrence order don't change. Normal conversation appending preserves both.
- **System blocks**: Potentially unstable. Claude Code injects varying `<system-reminder>` tags per request.

**For the common case** (Claude Code appending new turns to a growing conversation), block IDs are deterministic and stable. The `content_index` brittleness is a theoretical concern for multi-block messages with internal reordering, which doesn't normally occur in Claude Code's output.

**When IDs DO rotate:** Claude Code's compaction (when it hits ~95% context) rewrites message content. This would change content fingerprints for affected blocks, breaking persistent archive intent. However, this is a natural boundary — compaction fundamentally changes the conversation content, so archived blocks no longer make semantic sense anyway.

---

## 2. Root-Cause Proof Per Finding

### CRITICAL-1: Archive ≠ Payload Reduction

| Layer | What happens | Evidence |
|-------|-------------|---------|
| Planner projection | Sums block tokens → claims -61k | `validation.rs:241-248` |
| Applicator | Checks full-turn coverage → 0 turns removed | `applicator.rs:212-223` |
| Rewriter | `has_payload_changes()` → false → returns None | `rewriter.rs:148-156` |
| Engine | `archive_block_internal()` removes blocks from store | `rewriter.rs:253-254` |
| Handler | Forwards original body (rewriter returned None) | `handler.rs:426` |
| API | Receives full unmodified payload | JSONL L91: 134,574 cache_read tokens |

**The architecture contract is broken:** Projection promises block-level savings. Rewriter delivers turn-level removals. These are different things. In tool-heavy conversations (the most common Aperture use case), most turns contain a mix of tool_use, tool_result, and text blocks — partial-turn archival is the norm, not the exception.

### CRITICAL-2: Cleanup Name Mismatch

| Layer | Check | Result |
|-------|-------|--------|
| `is_context_tool_name("mcp__aperture__aperture_context_preview")` | `starts_with("aperture_context_")` | `false` |
| First pass (collect IDs) | Empty set | No IDs collected |
| Second pass (strip tool_use) | Nothing to strip | tool_use blocks remain |
| Third pass (strip tool_result) | Empty ID set | tool_result blocks remain |

### HIGH-1: Session Flip Sequence

1. Primary ingest → S1 active → UI shows S1 blocks
2. Haiku classifier ingest → S2 created, S2 active → UI gets S2 blocks
3. `notifyBlockMutations(S1_blocks, S2_blocks)` → all S1 IDs "missing" → false "archived" toast
4. Primary ingest resumes → S1 active → UI shows S1 blocks again

---

## 3. Fix Acceptance Criteria Per Finding

### CRITICAL-1: Archive Must Reduce Payload

**Must be true before calling it resolved:**
1. When archiving N blocks from a turn that has M total blocks (N < M), the payload MUST be reduced — either by content replacement (e.g., `[archived]` stub) or by removing individual content blocks from the turn.
2. Projection must report feasible savings that match actual payload reduction (within 10%).
3. Archival of 7 tool_result blocks totaling ~61k tokens must produce a measurable reduction in `cache_read_input_tokens` on the next API request.
4. Engine budget and `/context` must move in the same direction after archival (allowing for the structural gap from Claude-internal categories).
5. Regression test: stage/commit partial-turn archive → verify payload bytes shrink → verify `/context`-equivalent metric drops.

### CRITICAL-2: Cleanup Must Match Namespaced Names

**Must be true before calling it resolved:**
1. `is_context_tool_name("mcp__aperture__aperture_context_preview")` returns `true`.
2. All 8+ call sites inherit the fix from the single matcher function.
3. Cleanup strips both `mcp__aperture__aperture_context_*` and canonical `aperture_context_*` names.
4. Regression test with real Claude Code namespaced names passes.
5. No false positives: non-Aperture MCP tools (e.g., `mcp__github__create_issue`) are not stripped.

### HIGH-1: Session Flips Must Not Affect UI

**Must be true before calling it resolved:**
1. Auxiliary session creation does NOT change the active session.
2. OR: UI toast logic guards against session-switching (compares blocks only within the same session ID).
3. Block disappear/reappear during tool-use sub-requests is eliminated or explicitly marked as "loading."
4. Regression test: interleaved primary + auxiliary ingests → UI blocks remain stable.

### HIGH-2: Token Metrics Must Be Framed Clearly

**Must be true before calling it resolved:**
1. UI, MCP tools, and any user-facing metric report the SAME number for the same thing (either all include overhead or all exclude it — pick one).
2. Divergence from Claude `/context` is acknowledged as expected and NOT shown as an error.
3. Consider: remove or de-emphasize the comparison entirely, since the domains are structurally different.

---

## 4. What I Disagree With (vs. Round-2)

### Disagreement 1: Severity Classification

Round-2 classified projection mismatch as "P0" and cleanup mismatch as "P0." I disagree with the implicit equal-weight framing.

**CRITICAL-1 (projection/payload mismatch) is categorically more severe** than CRITICAL-2 (cleanup naming). The projection issue means *the entire archival pipeline produces zero value* — not just a token-waste problem, but a fundamental correctness failure that makes the product appear broken to users. CRITICAL-2 is a genuine bug with real token waste, but the system still *functions* (just with overhead).

### Disagreement 2: "Streaming Gate" Hypothesis

The JSONL analysis agent initially hypothesized that the rewrite doesn't run for streaming requests. **This is incorrect.** The rewrite runs at REQUEST time (`handler.rs:409-437`), before forwarding to upstream, regardless of whether the response will be streamed. The streaming gate (`!parsed.stream`) only applies to RESPONSE interception (`handler.rs:222`) — which handles context tool dispatch, not payload rewriting. This matters because it means the rewrite IS running but producing no changes (the correct diagnosis), not that it's being skipped.

### Disagreement 3: Block ID Churn as Primary Cause

Round-2 analysis and RESUME.md give significant weight to "block ID churn" as a root cause. My independent analysis shows **block IDs are actually quite stable for the common case** (Claude Code conversation growth). The IDs only rotate under specific edge conditions (compaction, system-reminder changes, content-array reordering). The primary reason archival doesn't work isn't ID instability — it's the turn-level removal constraint (CRITICAL-1).

---

## 5. What Was Ruled Out

1. **"Rewrite doesn't run for streaming requests"**: Ruled out. Rewrite runs at request time for all requests. Confirmed by code path tracing (`handler.rs:396-437`).

2. **"Engine captures pre-rewrite raw payload"**: Ruled out by round-1. Handler captures from effective body after rewrite path (`handler.rs:440-470`).

3. **"Block IDs are random/unstable by design"**: Ruled out. Block IDs are deterministic content-based hashes (`parser/mod.rs:153-170`). Identical content produces identical IDs.

4. **"Stale binary was running during latest repro"**: For the canonical repro (`66dd683a`), this is not applicable — the issue reproduces regardless of binary freshness because it's a design-level problem, not a timing bug.

5. **"Tool injection causes the budget divergence"**: Ruled out. `inject_tools()` is a NO-OP for `ClaudeMcpRuntime` (Claude Code path). Tool overhead comes from Claude Code's own tool definitions, not Aperture injection.

---

## 6. Open Questions

1. **Should partial-turn archival use content replacement instead of turn removal?** The design could replace archived blocks' content with `[archived: {summary}]` stubs within the turn, preserving turn structure while reducing tokens. This would require extending the applicator to emit `content_replacements` for partial-turn archives. Trade-off: Anthropic's cache would still be invalidated from the modification point, but subsequent requests with the same replacement applied would cache-hit.

2. **Does Claude Code's automatic `cache_control` interact with Aperture's payload rewriting?** Anthropic docs describe automatic caching (`"cache_control": {"type": "ephemeral"}` at request top level). If Claude Code uses this, Aperture's proxy should preserve it. Unknown: Does Claude Code set this, and does Aperture currently preserve or strip it?

3. **What is the actual frequency of auxiliary session ingests during normal usage?** The mechanism is proven, but the user-visible impact depends on how often Haiku classifier requests actually flow through the proxy. If they use a separate API endpoint that doesn't go through Aperture, the session flip issue is theoretical.

4. **Tool Search feature interaction**: Claude Code has a "Tool Search" feature that defers MCP tool loading when tool definitions exceed 10% of context. If this is active for Aperture's tools, they might not appear in every request's `tools` array, which would affect tool injection and cleanup behavior.

---

## 7. Proof Status Summary

| Finding | Status | Evidence Type |
|---------|--------|--------------|
| CRITICAL-1: Archive ≠ payload reduction | **PROVEN** | Code trace + replay test + JSONL evidence |
| CRITICAL-2: Namespaced cleanup miss | **PROVEN** | Code trace + replay test + JSONL evidence + official docs |
| HIGH-1: Session flip → false toasts | **PROVEN (mechanism)** | Code trace + unit tests. User-visible frequency needs live event trace. |
| HIGH-2: Token metric divergence | **PROVEN (structural)** | Design analysis + JSONL side-by-side. Not a bug — 3 different counting domains. |
| MEDIUM-1: Block ID stability | **PROVEN (stable for common case)** | Code trace + existing stability test (`parser/tests.rs:708`). Edge cases identified. |

---

## 8. Minimal Next Experiments Before Any Production Fix

### For CRITICAL-1 (Archive payload gap):

**Experiment A**: Extend the applicator to emit `content_replacements` for archived blocks in partial turns.
- Modify `apply_mutations()`: when a block is archived but its turn isn't fully covered, instead of doing nothing, add `(turn_index, block_content_index) → "[archived]"` to `content_replacements`.
- Write a failing test FIRST: stage partial-turn archive → assert `has_payload_changes() == true` and payload bytes shrink.
- Measure: before/after `cache_read_input_tokens` on next request.

**Experiment B** (alternative): Switch from turn-level removal to block-level content removal within turns.
- Modify the Anthropic payload rewriter to remove individual `content` array elements within a message (not entire messages).
- This is more invasive but more precise — archived tool_result content blocks can be removed without removing the entire user message.
- Risk: Must preserve Anthropic's alternating user/assistant turn constraint. Empty turns must be handled (minimum 1 content block per message).

### For CRITICAL-2 (Cleanup naming):

**Experiment**: Fix the matcher, add a regression test, measure cleanup behavior in a real session.
- Modify `is_context_tool_name()` to also match `contains("__aperture_context_")` or strip the MCP namespace prefix before checking.
- Write failing test: `is_context_tool_name("mcp__aperture__aperture_context_preview")` → assert true.
- Measure: after fix, verify cleanup strips all Aperture tool calls from a real Claude Code session.

### For HIGH-1 (Session flips):

**Experiment**: Guard `create()` against flipping active session for non-primary models.
- Option A: Don't set active session if the current active session exists and is a different model class (e.g., Opus vs Haiku).
- Option B: Add a `primary` flag to sessions; only primary sessions can become active.
- Write failing test: interleave primary (Opus) + auxiliary (Haiku) ingests → assert active session never flips to Haiku.

---

## 9. External Documentation Sources Consulted

All Aperture assumptions validated against official docs:

| Assumption | Source | Status |
|-----------|--------|--------|
| MCP tool naming: `mcp__servername__toolname` | [Claude Code MCP docs](https://code.claude.com/docs/en/mcp) | **CONFIRMED** |
| `/context` includes tools + system + memory + messages | [Charlie Gleason article](https://code.charliegleason.com/understanding-context-windows), [ClaudeLog FAQ](https://claudelog.com/faqs/what-is-context-command-in-claude-code/) | **CONFIRMED** |
| Cache keys cumulative prefix hashing | [Anthropic prompt caching docs](https://platform.claude.com/docs/en/docs/build-with-claude/prompt-caching) | **CONFIRMED** |
| Cache hierarchy: tools → system → messages | Same Anthropic docs | **CONFIRMED** |
| 20-block lookback per breakpoint | Same Anthropic docs | **CONFIRMED** |
| cache_create 12.5× cache_read (Opus 4.6) | Same Anthropic docs ($6.25 vs $0.50/MTok) | **CONFIRMED** |
| Min cacheable: 4096 tokens (Opus 4.5/4.6) | Same Anthropic docs | **CONFIRMED** |
| API is stateless (full messages[] every request) | [Anthropic Messages API docs](https://platform.claude.com/docs/en/build-with-claude/working-with-messages) | **CONFIRMED** |
| Each agentic loop iteration = separate API request | [How Claude Code Works](https://code.claude.com/docs/en/how-claude-code-works) | **CONFIRMED** |

**New finding from docs:** Anthropic supports automatic caching (`"cache_control": {"type": "ephemeral"}` at request top level). Claude Code likely uses this. Aperture should verify it preserves this field during payload rewriting.

---

## 10. Diagnostic Test Validation

All round-2 diagnostic tests confirmed passing (568 lib tests total):

```
cargo test --manifest-path src-tauri/Cargo.toml --lib
  engine::planner::tests::test_replay_projection_overstates_payload_savings_when_archive_set_is_partial_per_turn ... ok
  engine::planner::cleanup::tests::test_strip_anthropic_namespaced_mcp_context_tools_are_not_matched_currently ... ok
  engine::tests::test_ingest_auxiliary_session_flips_active_session_until_primary_ingests_again ... ok
  test result: ok. 568 passed; 0 failed

npx vitest run src/lib/stores/context-budget.test.ts
  8 tests passed
```
