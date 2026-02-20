# Fix Plan: Block ID Stability + Persistent Archival Correctness

## Context

Manual test runs (sessions `df4ad515`, `1baf6b88`, `8d599bf0`) consistently show: blocks disappearing during tool calls, committed archives not persisting, context % oscillating wildly, unknown block ID errors, and token mismatch. Deep code analysis reveals these all stem from one architectural flaw: **block IDs are position-dependent, and the archival pipeline cannot see the current request's blocks**.

---

## Root Cause Chain (3 broken links)

The archival pipeline has 3 chained failures:

### Link 1: Block IDs rotate on every ingest
`stable_block_id()` (`parser.rs:333`) uses `turn_index` (message array index) in the hash seed. Capture happens on the REWRITTEN body (`handler.rs:632-637`). When the rewriter strips archived turns, all subsequent `turn_index` values shift → new block IDs → persistent archival targets stale IDs.

### Link 2: Persistent archival re-apply can't find archived blocks
`plan_for_session()` (`planner/mod.rs:544`) checks `input.blocks` (engine blocks) for persistent archival re-apply. But archived blocks were removed from the engine by `archive_block_internal()` → they're not in `input.blocks` → re-apply mutations never generated.

### Link 3: Applicator can't map archived block IDs to turn indices
`apply_mutations()` (`applicator.rs:61`) builds `block_by_id` from engine blocks. Archive mutations for persistent-archived IDs → `block_by_id.contains_key()` returns false → mutation silently skipped → turn never removed from JSON.

**Net effect**: Committed archives work for exactly one request, then fail permanently. The LLM's full conversation reappears, the same blocks get re-suggested for archival, context % oscillates.

### Additional bugs

- **`#` prefix not stripped** in `validate_plan()` → LLM copies display-formatted IDs, validation rejects them
- **`blocks_captured` fires before ingest** → if regressive guard skips ingest, frontend never gets corrective event
- **`overhead_tokens` only counts tool array** → system message (~6.8k tokens) not included → ~18% budget mismatch

### Additional findings from deep exploration

- **`BlocksCaptured` event data is unused by frontend** — frontend only extracts model/provider metadata, discards actual blocks. All block updates come from `ContextUpdated` → IPC fetch cycle. (Not a bug, but means `blocks_captured` event ordering matters less than initially thought.)
- **`refresh_active_session_totals()` correctly updates `session.block_ids`** after `archive_block_internal()` — filters to only IDs in store. No latent regression from stable IDs.
- **Autonomous heuristics are already disabled** (`planner/mod.rs:563`) — LLM controls all mutations via staged planning. No death spiral from autonomous archival.
- **MCP session affinity has edge cases** around session GC and HTTP errors, but these are low priority and not causing current bugs.

---

## Fix 1: `#` Prefix Normalization (trivial, do first)

Strip leading `#` from block IDs at plan validation and application boundaries.

### Files

**`src-tauri/src/engine/planner/mod.rs`** — `validate_plan()` (line 657):
- Add helper: `fn normalize_block_id(id: &str) -> &str { id.strip_prefix('#').unwrap_or(id) }`
- Normalize all block IDs in `actions.archive`, `actions.recall`, `actions.pin`, `actions.unpin`, `actions.expand`, `actions.shift_to`, `actions.compress` before lookup against `block_ids` HashSet

**`src-tauri/src/engine/planner/applicator.rs`** — `apply_mutations()` (line 57):
- Normalize `block_id` in each mutation match arm before `block_by_id`/`request_by_id` lookup (defense in depth)

**`src-tauri/src/metacog/tools.rs`** — `normalize_plan_arguments()`:
- Extend to strip `#` prefix from all ID string values in archive/recall/pin/unpin/expand arrays

**Tests** (3):
- `validate_plan()` accepts `#`-prefixed IDs
- `apply_mutations()` matches `#`-prefixed IDs
- Mixed `#`-prefixed and bare IDs in same plan

---

## Fix 2: Content-Fingerprint Block IDs (core fix)

Replace `turn_index` with a content fingerprint in the block ID hash seed, making IDs stable across message insertions/removals.

### Files

**`src-tauri/src/proxy/parser.rs`** — `stable_block_id()` (line 333):
- Change signature: `fn stable_block_id(role: Role, provider: &str, content_fingerprint: &str, block_key: &str)`
- New seed: `format!("{provider}|{role:?}|{content_fingerprint}|{block_key}")`
- `content_fingerprint` = `short_hash()` of first 200 chars of block content (existing `short_hash` at line 429)

**`src-tauri/src/proxy/parser.rs`** — `make_block()` (line 352) / `make_tool_block()` (line 391):
- Add `content_fingerprint: &str` parameter (replaces `turn_index` in ID generation)
- Keep `turn_index: u32` as separate parameter for `metadata.turn_index` and `last_referenced_turn`

**`src-tauri/src/proxy/parser.rs`** — `parse_anthropic_blocks()` / `parse_openai_blocks()`:
- Before each `make_block()` call, compute: `let fingerprint = short_hash(&content.chars().take(200).collect::<String>());`
- Track `HashMap<String, u32>` for occurrence counting: key = `"{role:?}|{fingerprint}"`, value = count so far
- Append occurrence to `block_key`: `block_key = format!("{base_key}:{occurrence}")` for duplicate-content disambiguation
- For tool blocks: `tool_use_id` in `block_key` already provides uniqueness; fingerprint adds extra stability

**Tests** (5):
- Same content at different array indices → same block ID
- Two identical-content blocks → different IDs (occurrence counter)
- Removing a middle message → surrounding block IDs unchanged
- Adding a message → existing block IDs unchanged
- Tool_use blocks retain stable ID when preceding messages shift

---

## Fix 3: Pre-Rewrite Block Parsing + Plumbing (enables persistent archival)

Parse blocks from the ORIGINAL request body BEFORE rewriting, and pass them through the planner and applicator. This fixes both broken links (2 and 3) in the archival chain.

### Why this is needed

With stable IDs, `persistent_archived_ids` will contain valid block IDs that match across requests. But:
- The planner re-apply check (`planner/mod.rs:544`) uses engine blocks → archived blocks aren't there
- The applicator turn_index lookup (`applicator.rs:61`) uses engine blocks → archived blocks can't be mapped

Both need the CURRENT REQUEST's blocks (which include everything, including content the LLM re-sent that was previously archived).

### Files

**`src-tauri/src/proxy/handler.rs`** — `forward_request()` (~line 610):
- BEFORE calling `rewrite_request()`, parse the original body into blocks: `let pre_rewrite_blocks = parser::parse_request_blocks(body_for_processing, &provider)?;`
- Pass `pre_rewrite_blocks` to `rewrite_request()` as a new parameter

**`src-tauri/src/proxy/parser.rs`** — new public function:
- `pub fn parse_request_blocks(body: &[u8], provider: &str) -> Result<Vec<Block>>` — thin wrapper that parses blocks from a raw JSON body using existing `parse_anthropic_blocks()` / `parse_openai_blocks()` logic
- Reuses ALL existing parser internals (stable_block_id, make_block, etc.)

**`src-tauri/src/proxy/rewriter.rs`** — `rewrite_request()` (line 34):
- Add `request_blocks: &[Block]` parameter
- Build `request_block_ids: HashSet<String>` from these blocks
- Pass to `PlannerInput` as new field
- Pass `request_blocks` to `apply_mutations()` as new parameter

**`src-tauri/src/engine/planner/types.rs`** — `PlannerInput`:
- Add field: `pub request_block_ids: HashSet<String>` — block IDs from current request parse

**`src-tauri/src/engine/planner/mod.rs`** — `plan_for_session()` (line 544):
- Change: `let active_ids = input.request_block_ids.iter().map(|s| s.as_str()).collect();`
- Instead of: `let active_ids = input.blocks.iter().map(|b| b.id.as_str()).collect();`
- This means persistent_archived_ids are checked against the CURRENT REQUEST (which has the archived content), not engine blocks (which don't)

**`src-tauri/src/engine/planner/applicator.rs`** — `apply_mutations()` (line 57):
- Change signature: `pub fn apply_mutations(engine_blocks: &[Block], request_blocks: &[Block], mutations: &[ContextMutation]) -> RewriteDecisions`
- Build TWO lookup maps:
  - `engine_by_id` — for Compress/Expand/UpdateContent (need engine metadata like `compressed_versions.original`)
  - `request_by_id` — for Archive (these blocks exist in request but not engine)
- **Archive path** (line 68): check `request_by_id` instead of `block_by_id`
- **Compress/UpdateContent** (lines 77-107): use `request_by_id` for `turn_index` (matches current JSON payload), `engine_by_id` for content metadata
- **Expand** (line 108): use `engine_by_id` for `compressed_versions.original.content`, `request_by_id` for `turn_index`
- **Turn grouping** (lines 153-174): iterate `request_blocks` instead of `blocks` — ensures all blocks at each turn_index are visible (including blocks being archived)
- **Shift/Pin/Unpin**: engine-only updates, no turn_index needed, unchanged

**Tests** (4):
- Persistent archival re-apply: block in `persistent_archived_ids` + in request_blocks but not engine → generates Archive mutation
- `apply_mutations` with `request_blocks` containing archived block → `remove_turns` populated correctly
- apply_mutations: Expand uses engine original content + request turn_index
- Turn grouping with mixed engine/request blocks

---

## Fix 4: Event Ordering + IngestResult.applied (P2)

### Files

**`src-tauri/src/engine/mod.rs`** — `IngestResult`:
- Add `pub applied: bool` field
- Set `applied: false` in early return at line 210 (regressive guard)
- Set `applied: true` at normal return

**`src-tauri/src/proxy/handler.rs`** — `finalize_exchange()` (line 271):
- Move `dispatcher.blocks_captured(exchange)` to AFTER `engine.ingest()` call
- Gate on `IngestResult.applied`: only emit when ingest actually replaced blocks
- `dispatcher.response_complete()` stays before ingest (HTTP-level event, not engine state)

**Tests** (2):
- `IngestResult.applied` false for regressive captures
- `IngestResult.applied` true for normal ingests

---

## Fix 5: System Message Overhead (P2)

### Files

**`src-tauri/src/proxy/parser.rs`** — `estimate_tool_overhead()` (line 408):
- Rename to `estimate_request_overhead()`
- Also extract system message byte length and estimate tokens (bytes / 4)
- For Anthropic: `raw.get("system")` string or content-block array
- For OpenAI: `messages[0]` where role=system, or `raw.get("instructions")`
- Return total: tool_tokens + system_tokens

**Tests** (3):
- Request with system + tools → overhead includes both
- Request with no tools → overhead = system only
- Request with no system → overhead = tools only

---

## Implementation Order

| Step | Fix | Files | Complexity |
|------|-----|-------|-----------|
| 1 | `#` prefix normalization | planner/mod.rs, applicator.rs, tools.rs | Trivial |
| 2 | Content-fingerprint IDs | parser.rs | Medium |
| 3 | Pre-rewrite parsing + plumbing | handler.rs, parser.rs, rewriter.rs, types.rs, planner/mod.rs, applicator.rs | Medium-High |
| 4 | Event ordering + applied flag | engine/mod.rs, handler.rs | Low |
| 5 | System overhead | parser.rs | Low |

Fixes 2 and 3 are tightly coupled — stable IDs without the plumbing fix won't help because persistent archival still can't find the blocks.

---

## Verification

### Automated
- `cargo test --manifest-path src-tauri/Cargo.toml` — all existing + ~17 new tests pass
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` — clean
- `npx vitest run` — frontend tests pass (no frontend changes in this plan)

### Manual test (critical)
1. `aperture start` then `aperture claude`
2. Have a conversation with 5+ tool calls (file reads, greps)
3. Call `aperture_context_preview` → verify block IDs are stable (same content = same ID across calls)
4. Call `aperture_context_plan` with `{op: "stage", archive: ["#abc123"]}` → verify `#` prefix accepted
5. Call `aperture_context_plan` with `{op: "commit"}` → verify commit queued
6. Continue conversation (2-3 more turns) → verify:
   - Archived blocks stay archived (not re-appearing)
   - Context % doesn't oscillate (should decrease after archival, stay stable)
   - No block disappear/reappear during tool subrequests
   - Budget % within ~10% of `/context`
7. Check proxy logs for `"Skipping regressive subset ingest"` — should only appear for genuine partial captures, not after archival
