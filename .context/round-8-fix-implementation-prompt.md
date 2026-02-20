# Round 8 Fix Implementation

Read `.context/RESUME.md` first, then `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-8-investigation-2026-02-19.md` for full context.

## Task

Implement the verified fixes for Round 8 bugs. All root causes have been confirmed through code analysis and JSONL forensics. The investigation is complete — go straight to implementation.

## Fix Order: F2 → F3 → F1 → F4 → Fix B → F6 → F5

### F2: Guard thinking blocks in stub replacement (TRIVIAL)
**File**: `src-tauri/src/proxy/rewriter/payload.rs:316-348`
**What**: Add `"thinking" | "redacted_thinking" => { return; }` arm to the match in `replace_content_block_with_stub()`, before the `_ =>` catch-all.
**Test**: Add test that thinking/redacted_thinking blocks are untouched by stub replacement.

### F3: Never archive thinking blocks (SMALL)
**File 1**: `src-tauri/src/engine/planner/heuristics.rs:334-372` — add `Role::Thinking` exclusion in `is_archival_candidate()`
**File 2**: `src-tauri/src/engine/planner/validation.rs:45-54` — reject `Archive` mutations targeting `Role::Thinking` blocks in `validate_plan()`
**Test**: Add tests for both: thinking blocks excluded from candidates, plan validation rejects thinking archival.

### F1: Reorder pipeline — stubs before removal (SMALL)
**File**: `src-tauri/src/proxy/rewriter/payload.rs:10-31`
**What**: Change `apply_decisions_to_json()` from `remove → replace → stubs` to `stubs → replace → remove`. This is safe because stubs/replacements modify content WITHIN messages while removal removes entire messages. They target different turns (applicator ensures this at `applicator.rs:282-284`).
**IMPORTANT**: Apply the same reorder for ALL three API formats (Anthropic, OpenAI Chat, OpenAI Responses).
**Test**: Add test with both full-turn removals AND partial-turn stubs — verify stubs land on correct messages after removal. The existing tests should still pass.

### F4: Never merge assistant messages with thinking blocks (SMALL)
**File**: `src-tauri/src/proxy/rewriter/sanitize.rs:213-236`
**What**: In the merge loop, before merging consecutive assistant messages, check if either message contains `thinking` or `redacted_thinking` type blocks. If so, skip the merge. Instead, advance the write pointer (treat as different-role). The subsequent "ensure last message is user" pass will handle the structural fix if needed, OR insert a synthetic user message between them to break the adjacency.
**Test**: Add test with consecutive assistant messages where one has thinking blocks — verify they are NOT merged. Update the existing `test_structure_sanitizer_preserves_thinking_blocks_during_merge` to verify the messages stay separate.

### Fix B: Filter context tool blocks from engine ingest (SMALL)
**File**: Likely `src-tauri/src/proxy/handler/` or `src-tauri/src/engine/ingest.rs` — between parse and ingest
**What**: After parsing, filter out blocks where `metadata.tool_name` matches aperture context tools (`is_context_tool_name()` from `metacog/runtime.rs`). These blocks shouldn't accumulate in the engine's block store.
**Test**: Verify context tool blocks don't inflate block count in engine.

### F6: Tokenize search queries per-term (SMALL)
**File**: `src-tauri/src/metacog/tools.rs:575-607`
**What**: In `search_score()`, split `query_lower` by whitespace into individual terms. Score each term against content/file_paths/role/tool_name/topic_keywords. Add bonus for matching multiple terms. Also update `extract_search_snippet()` to work with individual terms.
**Test**: Add test with multi-word query matching blocks that contain individual terms but not the full phrase.

### F5: Better error for unknown plan parameters (SMALL)
**File**: `src-tauri/src/metacog/tools/plan.rs`
**What**: In `context_plan()` or `normalize_plan_arguments()`, detect unrecognized top-level keys (anything not in `archive/expand/recall/pin/unpin/shift_to/compress/split/control`). If found, return an error listing all expected parameter names with brief descriptions.
**Test**: Add test sending `{"query": "..."}` to plan tool — verify error lists expected params.

## Guidelines

- Write tests alongside each fix, not after
- Run `cargo test --manifest-path src-tauri/Cargo.toml` after each fix
- Run `cargo clippy --manifest-path src-tauri/Cargo.toml` after all fixes
- The investigation report has pseudocode for each fix — use as reference but adapt to the actual code
- Don't over-engineer. Minimal changes to fix each bug.
