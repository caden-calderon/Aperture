# Round 6 Fix Implementation

Read and execute exactly: this prompt. Implement 3 fixes for bugs found in Round 6 diagnostics.

## Context

Read these files first, in order:
1. `.context/RESUME.md` — current state
2. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-6-2026-02-19.md` — full root cause analysis with code path traces
3. `dev/active/phase-4-compression-readiness/tasks.md` — task checklist

## Fixes to Implement (in order)

### Fix 1: BUG #3 — serde_json preserve_order (trivial)

**Root cause**: `serde_json` without `preserve_order` uses BTreeMap, sorting JSON keys alphabetically on round-trip. This corrupts thinking block integrity (Anthropic rejects with "thinking blocks cannot be modified").

**Change**: In `src-tauri/Cargo.toml`, change line 20 from:
```toml
serde_json = "1"
```
to:
```toml
serde_json = { version = "1", features = ["preserve_order"] }
```

`indexmap` is already in the dependency tree (no new deps). Run `cargo test` + `cargo clippy` after.

### Fix 2: BUG #1 — Post-cleanup structural sanitizer (medium)

**Root cause**: `strip_anthropic_context_tools()` in `engine/planner/cleanup.rs` removes context tool_use from assistant messages and matching tool_result from user messages. User messages that had ONLY context tool_results become empty → removed. This leaves:
- Consecutive assistant messages
- Payload ending with assistant message (Anthropic rejects as "prefill")

**Where**: `src-tauri/src/proxy/rewriter/sanitize.rs`

**New function**: `sanitize_anthropic_message_structure(request_json: &mut Value) -> usize`

This function must:
1. **Merge consecutive same-role messages**: If two adjacent messages have the same role, merge their content arrays (or concatenate string content with `\n\n`). This handles the case where cleanup leaves `[assistant, assistant]`.
2. **Ensure last message is user**: If the last message is role=assistant, append a minimal synthetic user message: `{"role": "user", "content": [{"type": "text", "text": "Continue."}]}`. This prevents the "prefill" error.
3. Return the number of merges + synthetic messages added.

**Wire it**: In `src-tauri/src/proxy/rewriter.rs`, call `sanitize_anthropic_message_structure` AFTER the existing orphan sanitizers (after line 224, before re-serialization at line 230). Only for `is_messages_path(path)`.

Also wire it in the cold-start path (after the orphan sanitizers around line 91).

**Tests** (add to sanitize.rs or a test file):
1. No-op when messages alternate correctly and end with user
2. Merges two consecutive assistant messages into one
3. Merges three consecutive assistant messages into one
4. Appends synthetic user when last message is assistant
5. Handles the full cleanup scenario: `[user, assistant+tool_use, user(tool_result_only), assistant+tool_use, user(tool_result_only)]` → after cleanup → `[user, assistant, assistant]` → after sanitizer → `[user, assistant(merged), user("Continue.")]`
6. Preserves thinking blocks during merge (thinking blocks stay in content array)
7. No-op for OpenAI paths (function only runs for Anthropic)

**BUG #2 is fixed by this** — no separate implementation needed.

### Fix 3: BUG #5 — Stable system block fingerprint (low)

**Root cause**: System prompt content starts with `x-anthropic-billing-header:` which changes per request. `content_fingerprint()` hashes first 200 chars including this dynamic header → different block ID each request.

**Where**: `src-tauri/src/proxy/parser/anthropic.rs`

**Change**: In `parse_anthropic_request()`, before calling `content_fingerprint(&system_content)` (around line 115), filter out billing header lines:

```rust
// Normalize system content for stable fingerprinting by filtering
// dynamic headers that change per request (e.g. billing headers).
let fingerprint_content: String = system_content
    .lines()
    .filter(|line| {
        !line.trim_start().to_ascii_lowercase()
            .starts_with("x-anthropic-billing-header:")
    })
    .collect::<Vec<_>>()
    .join("\n");
let fp = content_fingerprint(&fingerprint_content);
```

This reuses the same filtering pattern already in `normalize_regression_content()` (ingest.rs:257-266).

**Important**: The `system_content` variable passed to `make_block()` must remain UNFILTERED (the full content including billing header). Only the fingerprint input is filtered. The block stores the real content; only the ID generation is normalized.

**Tests**:
1. Two system contents that differ only in billing header produce the same block ID
2. Two system contents that differ in actual instructions produce different block IDs
3. System content with no billing header is unaffected

## Quality Gates

After all 3 fixes:
```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run check  # if any frontend touched (unlikely)
```

Report final test counts and any issues.

## Do NOT

- Change any other code beyond the 3 fixes described
- Refactor surrounding code
- Add comments to code you didn't change
- Skip tests for any fix
