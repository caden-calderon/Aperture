# Demo Bugfixes — Session Context (2026-02-27)

> **For the next agent**: Read `.context/RESUME.md` first for current state, then this file for full details.

---

## Goal

Fix bugs visible during hackathon demo. Five issues total — 3 fixed, 2 implementations pending.

---

## Bug 1: Blocks Disappear on Idle (FIXED + VERIFIED)

**Problem**: Every time Claude Code goes idle between turns (>15 seconds), all context blocks disappear from the UI, then reappear on the next API request.

**Root cause**: `src/lib/stores/connection.svelte.ts:79-94` — `checkIdle()` called `onSessionResetCb()` after 15s idle, which triggered `contextStore.clearBlocks()`.

**Fix**: Removed `onSessionResetCb()` from `checkIdle()`. Status transitions but blocks persist.

---

## Bug 2: Thinking Blocks Visible to Claude (FIXED + VERIFIED)

**Problem**: Thinking blocks appear in MCP tool output. Claude tries to archive them, gets validation error, wastes tokens.

**Fix**: Filtered `Role::Thinking` at both dispatch entry points in `src-tauri/src/metacog/tools.rs`. Verified in session `3b3963a2`: 7 thinking blocks generated, 0 visible in any preview.

**Additional fix**: Added `ThinkingStats` to explain the token gap. Preview/status now shows:
```
[3 thinking blocks (12.4k tokens) excluded from breakdown — included in budget %]
```

**Key finding**: Claude Code strips thinking from `messages[]` before sending to API. Thinking blocks are only in responses (ephemeral). Our filter is defense-in-depth.

---

## Bug 3: Block Flickering (FIXED)

**Problem**: Blocks intermittently disappear during active use.

**Root cause**: Stale proxy binary (Feb 24, 3 days old). `tauri dev` only builds main `aperture` binary, not `aperture-proxy`. Fish function didn't kill old proxy process.

**Fix**: Updated `aperture.fish` to kill proxy + Vite processes and rebuild proxy on `aperture start`.

---

## Bug 4: `/context` Clears Blocks (CODE WRITTEN — UNVERIFIED)

**Problem**: Running `/context` in Claude Code causes all context blocks to disappear. They return on next API request.

### Root Cause: System Prompt Being Archived

**Evidence from session `3b3963a2` ("This is a test"):**

| Preview | Line | Blocks | Budget | System Prompt |
|---------|------|--------|--------|---------------|
| 1st | 11 | 6 | 16% (32k) | `#d29cf205` (3.3k, Primacy) |
| 2nd | 76 | 14 | 18% (35k) | `#d29cf205` (3.3k, Primacy) |
| 3rd | 89 | 20 | 3% (5.6k) | **GONE — no Primacy zone** |

Between preview 2 and 3:
- User ran `/context` (line 81) — no API request
- User asked Claude to check again (line 84)
- Claude called `aperture_context_preview` (line 86)
- Result: system prompt vanished, budget dropped 35k→5.6k

**Investigation confirmed:**
- **No plan calls** in entire session (no `aperture_context_plan`)
- **Heuristics disabled** (`planner/mod.rs:608`: "Autonomous heuristics are now DISABLED")
- **Persistent archived IDs** are the suspected source — IDs from prior sessions may carry over

**The smoking gun** — `proxy/rewriter/payload.rs:45-47`:
```rust
if turns_to_remove.contains(&0) {
    json.as_object_mut().map(|obj| obj.remove("system"));
}
```
This removes the Anthropic `system` field when turn 0 is in `remove_turns`. The capture then ingests a body without a system prompt.

### Fix Needed (3 layers)

**Layer 1 — Validation** (`engine/planner/validation.rs`):
```rust
// In validate_plan(), archive section (after thinking block check):
if block.role == crate::engine::types::Role::System {
    errors.push(format!(
        "Block {nid} is a system prompt and cannot be archived"
    ));
    continue;
}
```

**Layer 2 — Persistent re-application** (`engine/planner/mod.rs:588-605`):
```rust
// In plan_for_session(), persistent archived loop:
for block_id in &persistent_archived {
    if active_ids.contains(block_id.as_str()) && !already_archived.contains(block_id) {
        // Skip system blocks — they must never be archived
        if let Some(block) = input.blocks.iter().find(|b| b.id == *block_id) {
            if block.role == Role::System {
                continue;
            }
        }
        mutations.push(ContextMutation::Archive { block_id: block_id.clone() });
    }
}
```

**Layer 3 — Payload guard** (`proxy/rewriter/payload.rs:45-47`):
```rust
if turns_to_remove.contains(&0) {
    warn!("Refusing to remove system prompt (turn 0) — this would break the API request");
    // Do NOT remove json["system"] — ever.
}
```

---

## Bug 5: Block IDs Unstable Across Archival (CODE WRITTEN — UNVERIFIED)

**Problem**: When archival removes early turns, block IDs change for ALL remaining blocks because the `OccurrenceTracker` resets per-parse. This causes Svelte's keyed `{#each}` to unmount/remount everything with 350ms transitions.

### Mechanism

`stable_block_id(role, provider, content_fp, block_key)` in `parser/mod.rs:153`:
- `content_fp` = hash of first 200 chars (deterministic for same content)
- `block_key` includes occurrence counter: `anthropic:text:0:{occ}`
- `OccurrenceTracker` counts `(role, fingerprint)` pairs, resets to 0 per parse
- When archival removes a block with fingerprint "abc" at occ=0, the remaining block with fingerprint "abc" shifts from occ=1 to occ=0 → new block_key → new ID

**Unique content IS stable** — `#ffaada21` survived across previews because no other block shares its fingerprint (occ always = 0).

### Fix: Content-Addressed Merge in `ingest()`

In `src-tauri/src/engine/ingest.rs`, after parsing new blocks but before replacing session blocks:

```rust
/// Match new blocks to existing session blocks by content to preserve IDs.
/// This prevents visual disruption when archival shifts occurrence counters.
fn stabilize_block_ids(new_blocks: &mut [Block], old_blocks: &[Block]) {
    use std::collections::HashMap;

    // Build pool of reusable IDs indexed by (role_str, content_prefix_200).
    let mut pool: HashMap<(String, String), Vec<String>> = HashMap::new();
    for old in old_blocks {
        let key = block_content_key(old);
        pool.entry(key).or_default().push(old.id.clone());
    }

    // Greedily match new blocks to old IDs.
    for block in new_blocks.iter_mut() {
        let key = block_content_key(block);
        if let Some(ids) = pool.get_mut(&key) {
            if let Some(reused) = ids.pop() {
                block.id = reused;
            }
        }
    }
}

fn block_content_key(block: &Block) -> (String, String) {
    let prefix: String = block.content.chars().take(200).collect();
    (format!("{:?}", block.role), prefix)
}
```

Call it in `ingest()` right after combining all blocks (line ~73) and before removing old blocks:
```rust
let all_blocks = ...;
let old_blocks = self.store.get_many(&old_block_ids);
stabilize_block_ids(&mut all_blocks, &old_blocks);
// ... continue with remove_many, insert_many
```

---

## Deployment Issues Found

### Fish function didn't kill `aperture-proxy`
- Only killed `aperture$`, `aperture-mcp`, `npm run tauri dev`
- **Fixed**: Added `"src-tauri/target/debug/aperture-proxy"` to cleanup patterns

### Fish function didn't kill Vite dev server
- Port 1420 conflict on restart
- **Fixed**: Added `"node.*vite.*dev"` and `"node.*svelte-kit"` to cleanup patterns

### `tauri dev` doesn't build `aperture-proxy`
- Only builds main `aperture` binary
- Binary was 3 days stale — no Rust fixes took effect
- **Fixed**: Added `cargo build --bin aperture-proxy` step to `aperture start`

### Proxy stderr goes to dead pipe
- After Tauri restarts, proxy's stderr pipe has no reader
- All tracing output silently discarded
- No file appender configured in `util.rs:init_logging()`
- **Not fixed yet** — needs `tracing-appender` crate or stderr redirect

---

## Session Fragmentation (discovered 2026-02-27, may be root cause)

When querying the proxy engine via IPC during debugging, found **18 separate sessions** — most with 1 block:
- Active session: `exchange_count: 5`, `block_count: 1` (just "foo")
- Real conversation data scattered across other sessions with different `fallback:` thread identities
- This is the H9 bug: `derive_thread_identity()` (parser/identity.rs) creates different hashes when the first user+assistant pair changes

**This may be a bigger contributor to "blocks disappearing" than Bugs 4/5.** If the active session keeps flipping to a small sub-agent session, the UI shows empty/near-empty context regardless of system prompt protection or ID stability.

**Investigate next session**: Is session fragmentation the real cause, or a red herring?

---

## Key File Locations

| File | What |
|------|------|
| `src/lib/stores/connection.svelte.ts` | Bug 1 fix |
| `src-tauri/src/metacog/tools.rs` | Bug 2 fix + thinking token note |
| `~/.config/fish/functions/aperture.fish` | Deployment fixes (now builds proxy + MCP) |
| `src-tauri/src/engine/ingest.rs` | Bug 5 fix: `stabilize_block_ids()` (WRITTEN) |
| `src-tauri/src/engine/planner/validation.rs` | Bug 4 Layer 1: reject System archival (WRITTEN) |
| `src-tauri/src/engine/planner/mod.rs` | Bug 4 Layer 2: skip system in re-application (WRITTEN) |
| `src-tauri/src/proxy/rewriter/payload.rs` | Bug 4 Layer 3: never remove turn 0 (WRITTEN) |
| `src-tauri/src/proxy/parser/identity.rs` | Thread identity derivation (session fragmentation source) |
| `src/lib/stores/context.svelte.ts` | DIAG logging (remove after fixes) |
| `src/routes/+page.svelte` | DIAG logging (remove after fixes) |

## Test Counts

- Rust: 632 passing, clippy clean
- Frontend: 53 passing

## Next Session Checklist

1. **Start proxy with logging**: `RUST_LOG=debug` + file redirect, or add `tracing-appender`
2. **Send ONE request** through proxy and examine full log — verify blocks are ingested
3. **Check session count** — if still fragmenting, fix `derive_thread_identity` or session resolution
4. **If ingest works but UI empty** → investigate SSE event path / frontend connection
5. **If ingest doesn't work** → investigate parser / regression guard
6. **Verify Bug 4/5 fixes** once basic ingest is confirmed working
7. Remove DIAG diagnostics after all verified
8. Return to hackathon prep / refactor
