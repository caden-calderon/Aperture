# Round 13 — Overhead Audit + Deep Diagnostic (2026-02-21, updated 2026-02-22)

## Context

Full investigation of 7 issues surfaced during R11/R12 testing. Code analysis + JSONL log analysis. Aperture proxy logs (tracing/stderr) were NOT captured to disk — `init_logging()` uses `fmt::layer()` with no file appender, so R9-DIAG traces are lost.

## Log Files Analyzed

| Session | File | Size | Lines | Start | Notes |
|---------|------|------|-------|-------|-------|
| R11 | `4676b048-...jsonl` | 747K | 139 | "Hey sonnet" | Hackathon demo, 1 cleanup round only |
| R12 | `27619bec-...jsonl` | 1010K | 228 | "sonnet 4.6" | 3 rounds: R1+R2 work, R3 fails |
| R10 | `e62f4ad9-...jsonl` | 1.1M | ~180 | "hello sonnet" | 3 rounds all worked (8+8+5=21 blocks) |

---

## Issue 1: Plan Layering — 3rd Committed Plan Never Fires

### Observation (from R12 JSONL)

| Round | Blocks | Commit result | Fired? | Breadcrumb |
|-------|--------|---------------|--------|------------|
| R1 | 4 | Success (-30k projected) | Yes | 4 archives, Net: -30k |
| R2 | 5 | Success (-40k projected) | Yes | 9 archives (4+5), Net: -40k |
| R3 | 13 | Success (-31k projected) | **No** | Still 9 archives, Net: +0 |

After R3 commit, 7+ subsequent turns all show breadcrumb with only 9 archives (R1+R2). The 13 R3 blocks remain at full size in all manifests. Context keeps growing (95k → 112k).

### Detailed R12 R3 Data (from JSONL deep analysis)

**R3 Stage (Line 174)**: 13 unique block IDs staged, all validated, -31k projected, no errors.

**R3 Commit (Line 180)**: Returned success, -31k projected, 87 blocks, 24% budget.

**No `_aperture_session_id` hints** found anywhere in the JSONL (these are in HTTP bodies, not in Claude Code's conversation log — expected).

**Zero overlap** between R3's 13 block IDs and R1/R2's 9 block IDs. All are unique targets.

**Post-R3 breadcrumbs**: EVERY subsequent turn shows exactly 9 archives (R1+R2 persistent), Net: +0, Budget: 22-29%. The 13 R3 block IDs NEVER appear in any breadcrumb.

**`/context` token count climbed monotonically after R3**: 80k → 96k → 100k → 111k. If R3 had fired (-31k), it should have dropped. No compaction occurred (context only grew, never shrank).

### Unit Test: PASSED

`test_three_round_plan_layering_r12_regression` in `engine/planner/tests.rs:1777` simulates exact R12 scenario (3 rounds: 4+5+13). **Test passes** — planner logic is correct in isolation. All assertions verified including R3: 13 from pending + 9 from persistent = 22.

Run with: `cargo test --manifest-path src-tauri/Cargo.toml --lib test_three_round_plan_layering`

### Code Analysis — Exhaustive Static Review

Every code path was traced in detail. The planner logic, session resolution, persistent archive merging, and re-application loop all look correct in isolation.

**Commit path** (`tools/plan.rs:245-256`):
- `commit_staged_plan_for_session(session_id)` — moves staged → pending
- `add_persistent_archives_for_session(session_id, mutations)` — inserts 13 IDs into HashSet
- Both use same `session_id` from `resolve_tool_session_id()`

**Consumption path** (`rewriter.rs:122-154`):
- `take_pending_plan_for_session(&session_id)` — consumes pending plan
- `plan_for_session(&session_id, &input)` — applies mutations + re-applies persistent archives
- Uses `session_id` from `engine.resolve_session(provider, model, "proxy", thread_identity)`

**Session ID resolution (TWO different paths)**:
- **MCP path** (`context_api.rs:224-246`): `resolve_tool_session_id()` → checks `_aperture_session_id` hint, falls back to `engine.active_session_id()`
- **Rewriter path** (`rewriter.rs:58-63`): `engine.resolve_session(provider, model, "proxy", thread_identity)` → identity-based lookup via `ensure_session()` → calls `switch_to()` to activate

### LEADING HYPOTHESIS: Ingest-Driven Session Divergence (H9)

**Discovery**: The `ingest()` function (`ingest.rs:26`) ALSO calls `ensure_session()`, which calls `switch_to()` to change the active session. Critically:

- The **rewriter** resolves its session from the **PRE-REWRITE** body's `thread_identity`
- The **ingest** resolves its session from the **POST-REWRITE** body's `thread_identity` (via the capture body)
- These parse DIFFERENT JSON — the post-rewrite body has turns removed, stubs inserted, tool blocks stripped, breadcrumbs appended, and structure sanitized

If the post-rewrite JSON produces a different `thread_identity` than the pre-rewrite JSON:
1. The ingest creates/activates a DIFFERENT session (S_ingest) than the rewriter used (S_rewriter)
2. `active_session_id()` now returns S_ingest
3. MCP plan operations (stage, commit) use `active_session_id()` → S_ingest
4. Plans stored under S_ingest, persistent archives added to S_ingest
5. Next proxy request: rewriter uses S_rewriter again → `take_pending_plan(S_rewriter)` → **None!**
6. Persistent archives for S_rewriter still have 9 (R1+R2 only) → breadcrumb shows 9

**This explains ALL observations**:
- R1 and R2 work because early turns have minimal rewriting (identity unchanged)
- R3 fails because by round 3, cumulative rewrites (9 archived turns removed, stubs, cleanups) are enough to change the thread_identity derived from the post-rewrite body
- The persistent set under S_rewriter stays at 9 forever (R3 additions went to S_ingest)
- The pending plan under S_ingest is never consumed (rewriter always uses S_rewriter)

**Why it's hard to confirm statically**: `fallback_thread_identity()` uses the first non-transient User block + first Assistant block content (first 160/120 chars). These SHOULD be stable (Primacy zone, not archived). But rewriter modifications (cleanup, sanitization, message structure merging) could subtly change what the parser sees as the "first" block. The identity derivation operates on BLOCKS produced by the PARSER from the JSON — if the JSON structure changes, the blocks change.

**Thread identity code**: `parser/identity.rs:107-176` — uses first `Role::User` (non-transient) + first `Role::Assistant` blocks.

**What could change the first blocks in post-rewrite JSON**:
- `cleanup_history()` strips tool blocks → could change message content arrays
- `sanitize_anthropic_message_structure()` merges consecutive same-role messages
- `sanitize_anthropic_orphan_tool_uses/results()` removes orphan blocks
- Turn removal creating consecutive same-role messages that get merged
- Any of these affecting the FIRST user or assistant message text

### Updated Hypothesis Table

| # | Hypothesis | Status | Evidence |
|---|-----------|--------|----------|
| H1 | Session ID mismatch (MCP vs rewriter) | **Subsumed by H9** | See H9 |
| H2 | Cold-start path (blocks empty) | **Unlikely** | R9-DIAG trace exists for this; blocks are non-empty after first ingest |
| H3 | add_persistent replaces instead of merges | **Ruled out** | Code verified, uses HashSet::insert |
| H4 | Block IDs stale / don't match request | **Ruled out** | Content fingerprints use first-200-chars hash + role + block_key. Stable across turns. `OccurrenceTracker` is position-independent for unique content. |
| H5 | Session drift after multiple cycles | **Subsumed by H9** | Drift comes from ingest, not from rewriter |
| H6 | Capacity issue at 22+ IDs | **Ruled out** | HashSet has no limit. R10 worked with 21. |
| H7 | Pending plan consumed by intermediate request | **Ruled out** | Claude Code is sequential. MCP commit completes before next API request. |
| H8 | plan_for_session writes back stale persistent set | **Ruled out** | Write-back only happens inside `if let Some(ref plan) = input.pending_plan` block. When pending_plan is None, no write-back occurs. |
| **H9** | **Ingest changes active session via ensure_session with different thread_identity from post-rewrite body** | **LEADING — needs confirmation** | Ingest uses capture body (post-rewrite). If thread_identity differs, active session diverges from rewriter's session. MCP then uses wrong session for plan storage. |

### What's Needed to Confirm H9

1. **Targeted integration test**: Simulate full flow (rewriter → capture parse → ingest → MCP commit → rewriter) with enough archival that the post-rewrite body differs. Check if `thread_identity` changes between pre-rewrite and post-rewrite parses.

2. **OR add diagnostic logging**: Log the session_id from both `resolve_session` (rewriter) and `ensure_session` (ingest) on every request. Run R14 test. If they diverge after N rounds of archival, H9 is confirmed.

3. **OR simplest fix**: Make the ingest use the SAME session_id as the rewriter (pass it through the capture/exchange path instead of re-resolving from the post-rewrite body). This would fix H9 regardless of whether it's the actual cause.

---

## Issue 2: Breadcrumb Re-Fires Every Turn (CONFIRMED)

### Root Cause (CONFIRMED from code)

The breadcrumb is generated inside `plan_for_session()` (planner/mod.rs:641-656) whenever the `mutations` vector is non-empty. The persistent archive re-application loop (lines 582-600) generates Archive mutations on EVERY turn for blocks that are in `persistent_archived_ids` and also in `request_block_ids`. This means:

1. Turn N: Commit fires → 9 archive mutations → breadcrumb generated ✓ (intended)
2. Turn N+1: No pending plan, but persistent re-application → 9 archive mutations → breadcrumb generated ✗ (unintended)
3. Turn N+2: Same → breadcrumb again ✗

**Evidence from R12**: 15+ breadcrumb appearances across the session, all showing the same 9 blocks with Net: +0 and slowly drifting Budget %.

### Impact

- **Cache invalidation every turn**: Breadcrumb modifies the last user message content → changes the trailing edge of the messages array → Anthropic cache miss for every request from the modification point onward.
- **Token overhead**: ~150 tokens per breadcrumb × every turn = significant accumulation.
- **Model confusion**: Model sees "[Context update: archived #X. Net: +0]" every turn, wasting reasoning on why it's +0.

### Fix Design

Distinguish "fresh" mutations (from a pending plan being consumed) from "persistent re-application" mutations. Only generate breadcrumb for fresh mutations.

**Option A**: Flag persistent re-application mutations with a marker, skip them in breadcrumb generation.
**Option B**: Only generate breadcrumb when `input.pending_plan.is_some()` (a plan was actually consumed this turn).
**Option C**: Track a "last_breadcrumb_turn" in PlannerSessionState, suppress breadcrumb if it was already generated on a recent turn.

**Recommended: Option B** — simplest, directly addresses the cause.

---

## Issue 3: MCP Tool Results Never Stripped (CONFIRMED)

### Root Cause (CONFIRMED from code)

`cleanup_history()` uses `is_intercepted_context_tool_name()` (runtime.rs:77-79) which ONLY matches bare-prefix tools (`aperture_context_*`), explicitly excluding MCP-namespaced tools (`mcp__aperture__*`).

```rust
pub fn is_intercepted_context_tool_name(name: &str) -> bool {
    name.starts_with(CONTEXT_TOOL_PREFIX) && !name.starts_with(MCP_CONTEXT_TOOL_PREFIX)
}
```

The design intent was: "MCP-namespaced tools are legitimate conversation entries and must be preserved so the model remembers calling them." But this means the full tool_use + tool_result blocks from every cleanup cycle persist in context forever.

### Impact (from R12 data)

- **16 MCP tool calls** generated ~63KB / ~16k tokens of results that persist forever
- At session end: **14% of context** was Aperture's own tool overhead
- Each `aperture_context_status` call grows with block count: 702 chars (7 blocks) → 6,824 chars (67 blocks)
- A cleanup cycle (preview + stage + commit) adds ~4-5k tokens that can never be reclaimed

### The Irony

The context management tools themselves are the largest source of unrecoverable context growth. Cleaning 30k tokens but adding 15k in tool overhead gives only 50% net savings.

### Fix Design

Strip MCP context tool blocks from conversation history after the model has seen the response. Specifically: strip `mcp__aperture__*` tool_use and tool_result blocks that are from turns OLDER than the current turn. The model has already processed the result; keeping it serves no purpose.

Change `is_intercepted_context_tool_name()` to `is_context_tool_name()` in the cleanup functions, or add a separate "stale MCP tool cleanup" pass that strips MCP context tool blocks older than current_turn - 1.

---

## Issue 4: Status Manifest O(n) Growth (CONFIRMED)

### Evidence

From R11: status result grows from 702 chars (7 blocks) to 6,824 chars (67 blocks) — **9.7× growth**. Each status/preview call adds this to context permanently (see Issue 3).

### Fix Design

- **Compact mode**: Return only zone summaries + total tokens, not individual block listings
- **Delta mode**: Only return what changed since last call
- Not a blocker, but compounds with Issue 3.

---

## Issue 5: Breadcrumb Net: +0 (CONFIRMED)

### Root Cause

`estimate_token_delta()` in planner/mod.rs calculates the delta from mutations. For persistent re-application, the blocks are already archived in the engine store, so the delta is 0. The FIRST time a plan fires, the delta is correct (e.g., -30k). On subsequent turns (persistent re-application), the delta is 0 because the engine already applied the archival.

### Fix

**Moot if Issue 2 is fixed** (no breadcrumb on re-application turns).

---

## Issue 6: Budget % Gap (CONFIRMED)

### Evidence

| Source | Aperture engine | Claude Code /context | Gap |
|--------|----------------|---------------------|-----|
| R12 L64 | 42% | 46% | 4% |
| R12 L124 | 51% | 59% | 8% |
| R12 L171 | 37% | 40% | 3% |

### Root Cause

Aperture's budget calculation only counts blocks in the engine store. Claude Code's /context includes overhead that Aperture doesn't track:
- System prompt: ~3.4k
- System tools: ~17.7k
- MCP tools: ~3.6k
- Memory: ~5.7k
- Compact buffer: ~3k
- **Total overhead: ~22-34k tokens** (varies)

### Fix

Include an overhead estimate in the budget calculation. Either:
- Hard-code an overhead constant (simple but fragile)
- Track overhead from parsed request (system blocks + tool definitions)

---

## Issue 7: Output Guardrail Hides Recency Blocks (CONFIRMED)

### Evidence

R12: Preview guardrail truncated 17, 53, 63 blocks across 3 calls. All truncated blocks are in the Recency zone. The model can only see Middle zone blocks in preview, making it impossible to target Recency blocks for archival.

### Fix Design

- Show zone summary for omitted blocks (e.g., "63 Recency blocks omitted, total ~45k tokens")
- Allow `aperture_context_preview` to accept a `zone` filter parameter

---

## Additional Findings

### A. Proxy Logs Not Persisted

`init_logging()` in lib.rs uses `tracing_subscriber::fmt::layer()` with no file appender. All R9-DIAG traces go to stderr and are lost when the terminal closes. **Must add file-based logging before next test.**

### B. Ghost 0-Token Block

R11: Block `#c624cab1` appears as 0-token assistant block from the first status call and persists throughout. Likely an empty assistant turn stub from the initial greeting.

### C. /context Command Costs ~900 Tokens

Each `/context` output adds ~828 tokens of formatted text to the conversation. Running it 5 times = ~4.5k tokens of diagnostic overhead.

### D. Proxy Crash (Non-File-Edit)

R11: Connection failed at 18:02 (519ms timeout). No file edits in this session — separate from the known file-edit crash bug.

### E. archive_block_internal REMOVES from store

`archive_block_internal()` at `engine/mod.rs:766-774` calls `self.store.remove(block_id)` — completely removes the block, doesn't just mark it. This is by design (stateless clients re-send content, ingest replaces store), but it means the engine store after archival doesn't contain archived blocks.

---

## Files Read During Investigation

| File | What was analyzed |
|------|-------------------|
| `engine/planner/mod.rs` | Full planner logic: plan_for_session, persistent re-application, commit, add_persistent_archives |
| `engine/planner/validation.rs` | validate_plan: checks block IDs against engine.session_blocks |
| `engine/planner/applicator.rs` | apply_mutations: converts mutations to JSON rewrite decisions |
| `engine/planner/cleanup.rs` | generate_breadcrumb: fires whenever mutations non-empty |
| `engine/planner/tests.rs` | Added R12 regression test (passes) |
| `engine/mod.rs` | ensure_session, resolve_session, archive_block_internal, switch_to |
| `engine/ingest.rs` | ingest: calls ensure_session (can change active session!) |
| `proxy/rewriter.rs` | Full rewriter flow: resolve_session, take_pending_plan, plan_for_session, apply |
| `proxy/handler.rs` | Full handler: pre-rewrite parse, rewrite, capture, forward, ingest |
| `proxy/handler/exchange.rs` | finalize_exchange: calls ingest with exchange.thread_identity |
| `proxy/context_api.rs` | resolve_tool_session_id: falls back to active_session_id |
| `proxy/parser/identity.rs` | Thread identity derivation: explicit + fallback |
| `proxy/parser/mod.rs` | content_fingerprint, stable_block_id, OccurrenceTracker |
| `proxy/capture.rs` | thread_identity stored from parsed (post-rewrite body) |
| `metacog/tools/plan.rs` | context_plan: normalize, validate, stage, commit flow |
| `mcp/server.rs` | MCP server: session affinity, with_plan_session_hint |
| `bin/aperture_mcp.rs` | Binary entrypoint |

---

## Next Steps (Priority Order)

1. **Confirm or rule out H9** (ingest session divergence):
   - Write integration test that exercises: pre-rewrite parse → rewrite → post-rewrite parse → check if thread_identity differs
   - OR add `warn!()` logging of session_id in both rewriter and ingest, run R14
   - OR simplest fix: pass rewriter's session_id through to ingest instead of re-resolving

2. **Fix Issue 2** (breadcrumb re-fire): Guard breadcrumb on `input.pending_plan.is_some()`

3. **Fix Issue 3** (MCP tool stripping): Strip `mcp__aperture__*` tool blocks from older turns

4. **Fix Issue 1** based on H9 confirmation: Make ingest use rewriter's session_id
