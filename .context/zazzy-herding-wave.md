# Phase 3 Remaining Checkpoints: E, F, G (Historical Implementation Log)

> Status update (2026-02-13): Checkpoints E/F/G are complete. Use `dev/active/metacog-dynamic-shifting/staff-review-2026-02-13.md` and `tasks.md` for current remediation work.

## Situation

Checkpoints A-D built the planner (mutations, manifest, heuristics, file tracking), runtime adapters (Claude MCP, Codex proxy, Passive), cleanup system (breadcrumbs, tool stripping), payload rewriter, and UI state animations. **But the proxy never actually injects tools or intercepts responses.** The full design doc lifecycle is:

```
Request arrives → inject tools → inject manifest → forward →
  ← response → extract context calls → dispatch internally →
    if ONLY context calls: re-invoke with results (inner loop)
    if mixed/none: forward response to client, apply mutations
```

Currently only the `inject manifest` and `cleanup history` steps run. Tools are never injected, responses are never intercepted, dispatch_tool is never called, engine block updates are computed but discarded, and there's no first-turn gating.

### Gap Inventory

| Gap | Where | Status |
|-----|-------|--------|
| `inject_tools()` never called | rewriter.rs | Missing |
| Response-side tool interception | handler.rs | Missing |
| Re-invoke inner loop | handler.rs | Missing |
| `dispatch_tool()` unused from proxy | metacog/tools.rs | Missing |
| Engine block updates discarded | rewriter.rs / applicator.rs | Missing |
| First-turn gating | rewriter.rs | Missing |
| Stream detection (`stream: true`) | parser.rs / rewriter.rs | Missing |
| Settings UI for budget ceiling | frontend | Missing |
| MCP server for Claude Code | standalone binary | Missing |
| Manual test playbook | docs | Missing |

### Streaming Strategy

The proxy detects streaming from the **response** `Content-Type: text/event-stream`, not from the request body. For the re-invoke loop to work, we need the full response body. Two options:

**v1 approach (this checkpoint):** Detect `stream: true/false` in the request JSON during parsing. Only inject tools when `stream: false` (or absent). When `stream: true`, skip tool injection — the model gets manifest + heuristics but no interactive tools. This is safe because:
- Streaming responses tee SSE chunks to the client in real-time — we can't intercept tool calls without either buffering the entire stream (breaking streaming UX) or post-hoc correction (complex, error-prone)
- Most coding tool requests that benefit from context management are non-streaming (tool calls are non-streaming by nature in most clients)
- The model still gets full autonomous heuristic support on streaming requests

**Future enhancement:** Buffer streaming tool call deltas, detect context-only tool calls, and re-invoke. This is Phase 4+ territory.

---

## Checkpoint E: Full Proxy Tool Lifecycle (CodexProxy)

The core integration — wiring the complete tool lifecycle through the proxy for non-streaming requests.

### E1. Stream Detection in Parser

**File:** `src-tauri/src/proxy/parser.rs` (MODIFY)

Add `stream` field to `ParsedRequest`:

```rust
pub struct ParsedRequest {
    pub provider: Provider,
    pub model: String,
    pub blocks: Vec<Block>,
    pub system_prompt: Option<String>,
    pub stream: bool,  // NEW
}
```

Extract from JSON body in all three parse functions:
- `parse_anthropic_request()` → `json["stream"].as_bool().unwrap_or(false)`
- `parse_openai_chat_request()` → `json["stream"].as_bool().unwrap_or(false)`
- `parse_openai_responses_request()` → `json["stream"].as_bool().unwrap_or(false)`

~3 tests: verify stream=true/false/absent parsing for each provider.

### E2. Tool Injection in Rewriter

**File:** `src-tauri/src/proxy/rewriter.rs` (MODIFY)

After manifest injection and cleanup, add tool injection step — gated on:
1. Runtime is NOT Passive (`runtime.kind() != RuntimeKind::Passive`)
2. Request is NOT streaming (`!parsed.stream`)
3. First-turn gate passes (see E3)

```rust
// After cleanup_history + inject_manifest:
if runtime.kind() != RuntimeKind::Passive && !parsed.stream && should_inject_tools(blocks) {
    runtime.inject_tools(&mut json);
}
```

~4 tests: tools injected for non-streaming Codex, tools NOT injected for streaming, tools NOT injected for passive runtime, tools NOT injected on first turn.

### E3. First-Turn Gating

**File:** `src-tauri/src/proxy/rewriter.rs` (MODIFY)

Add a `should_inject_tools()` helper:

```rust
fn should_inject_tools(blocks: &[Block]) -> bool {
    // Don't inject tools when context is small — it's overhead.
    // Threshold: more than 3 non-system blocks (i.e., at least 2 conversation turns)
    let non_system = blocks.iter().filter(|b| b.role != Role::System).count();
    non_system > 3
}
```

Design doc Open Question #2: "Only inject after context reaches some threshold where shifting becomes relevant." Starting with >3 non-system blocks (conservative — can tune later).

Manifest injection should still happen even when tools aren't injected (the status line is cheap at ~30 tokens), but only after the first exchange (at least 1 non-system block).

~3 tests: no tools/manifest on empty context, manifest but no tools on small context, both on mature context.

### E4. Response Interception for Non-Streaming

**File:** `src-tauri/src/proxy/handler.rs` (MODIFY)

Modify `handle_non_streaming_response()` to check for context tool calls:

```rust
async fn handle_non_streaming_response(
    state: &Arc<ProxyState>,
    request_id: &str,
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    upstream_response: reqwest::Response,
    request_was_captured: bool,
    request_start: Instant,
    upstream_latency: Duration,
    parsed: Option<&ParsedRequest>,    // NEW param
    path: &str,                         // NEW param
) -> Result<Response, ProxyError> {
    let response_bytes = upstream_response.bytes().await?;

    // Check for context tool calls in non-streaming response
    if let (Some(parsed), Some(ref engine)) = (parsed, &state.engine) {
        if let Some(rewritten_response) = interceptor::try_context_tool_interception(
            state, request_id, path, parsed, engine, &response_bytes, request_start
        ).await? {
            // Context tools were handled — return the re-invoked response
            return Ok(rewritten_response);
        }
    }

    // Normal path: capture, finalize, return
    // ... existing code ...
}
```

### E5. Context Tool Interception Logic

**File:** `src-tauri/src/proxy/interceptor.rs` (NEW)

Dedicated module for the tool interception + re-invoke loop:

```rust
pub async fn try_context_tool_interception(
    state: &Arc<ProxyState>,
    request_id: &str,
    path: &str,
    parsed: &ParsedRequest,
    engine: &ContextEngine,
    response_bytes: &Bytes,
    request_start: Instant,
) -> Result<Option<Response>, ProxyError>
```

**Logic:**
1. Parse response JSON
2. Select runtime via `detect_runtime()` + `select_runtime()`
3. Call `runtime.extract_context_calls(&response_json)` → `Vec<ContextToolCall>`
4. If empty → return `None` (no interception needed)
5. Dispatch each call via `metacog::tools::dispatch_tool()` → collect `Vec<ContextToolResult>`
6. Determine if response contains ONLY context tool calls (no real tool calls, no text content)
   - For ChatCompletions: check `choices[0].message.tool_calls` — all are context tools, no `content`
   - For Responses: check `output[]` — all items are context function_calls
7. If mixed (real + context calls): inject results into the original request body, strip context tool calls from response, return modified response to client
8. If context-only: **re-invoke** — build new request with original messages + context tool results injected, forward to upstream, recurse (with depth limit of 3)

**Re-invoke details:**
- Take the original request body (pre-rewriting, from the captured exchange)
- Inject context tool results via `runtime.inject_results()`
- Also append the assistant message with tool calls from the response
- Run through rewriter again (cleanup + manifest + tools)
- Forward to upstream
- Check response again for context calls (loop, max 3 iterations)
- Return final response to client

**Safety:**
- Max re-invoke depth: 3 (prevents infinite loops)
- Total re-invoke timeout: 60 seconds
- If any re-invoke fails: return original response to client (fail-open)

~10 tests:
- No context calls → returns None
- Context-only calls → dispatches + re-invokes
- Mixed calls → strips context calls from response, does NOT re-invoke
- Re-invoke depth limit respected
- dispatch_tool called with correct arguments
- Results injected correctly per format (ChatCompletions, Responses)
- Fail-open on re-invoke error

### E6. Apply Engine Block Updates

**File:** `src-tauri/src/proxy/rewriter.rs` (MODIFY)

After `apply_mutations()` returns `RewriteDecisions`, apply `engine_updates` to the engine:

```rust
// After apply_mutations():
for update in &decisions.engine_updates {
    match update {
        EngineBlockUpdate::ShiftZone { block_id, zone } => {
            engine.move_block_internal(block_id, zone.clone());
        }
        EngineBlockUpdate::SetPinned { block_id, pinned } => {
            engine.set_pin_internal(block_id, *pinned);
        }
    }
}
```

This requires adding lightweight internal mutation methods to `ContextEngine` that bypass policy checks (since these are system-driven, not user-driven). Add `move_block_internal()` and `set_pin_internal()` to `engine/mod.rs`.

~4 tests: zone shift applied to engine store, pin applied, unknown block_id skipped, updates visible in subsequent `active_session_blocks()`.

### E7. Module Registration + Imports

- Add `pub mod interceptor;` to `src-tauri/src/proxy/mod.rs`
- Update `forward_request()` in handler.rs to pass `parsed` and `path` to `handle_non_streaming_response()`
- Update `handle_streaming_response()` signature comment noting tools are not injected for streaming requests

### E8. Checkpoint E Tests Summary

| File | Tests | Focus |
|------|-------|-------|
| parser.rs | ~3 | stream field extraction |
| rewriter.rs | ~7 | tool injection gating, first-turn logic |
| interceptor.rs | ~10 | interception, dispatch, re-invoke loop |
| engine/mod.rs | ~4 | internal block updates |
| **Total** | **~24** | |

### E9. Manual Test: CodexProxy Tool Lifecycle

After implementation, verify with a real Codex CLI session through the proxy:

```bash
# Start Aperture
make dev

# In another terminal, launch Codex through proxy
OPENAI_BASE_URL=http://localhost:5400/v1 codex

# Test sequence:
# 1. Have a multi-turn conversation (5+ turns) to build context
# 2. Check proxy logs for: "Context rewriting applied" messages
# 3. Verify manifest appears in system message (grep proxy debug logs)
# 4. After ~5 turns, verify tools are injected (look for aperture_context_* in request logs)
# 5. Ask the model to "check your context" — it should call aperture_context_preview
# 6. Verify proxy logs show interception + dispatch + re-invoke
# 7. Ask it to archive old blocks — verify context_plan is called
# 8. On next turn, verify archived blocks are removed from payload
# 9. Check UI: blocks should show archived/compressed states
# 10. Budget bar should reflect real usage
```

---

## Checkpoint F: Settings UI + Integration Tests + Manual Test Playbook

### F1. Settings Panel Component

**File:** `src/lib/components/settings/SettingsPanel.svelte` (NEW)

A slide-out panel or modal containing:
- **Budget Ceiling Slider**: Range 40-100%, default 80%, step 5%
  - Live preview on TokenBudgetBar (ceiling marker moves)
  - Persists to localStorage (`aperture:budget-ceiling`)
  - Sends to engine via `invoke('engine_set_budget_ceiling', { ceiling })`
  - Shows derived thresholds: soft/medium/hard below slider
- **Runtime Info**: Read-only display of detected runtime (Codex/Claude/Passive)
- **Context Stats**: Total blocks, archived count, compressed count, token savings

### F2. Settings Toggle in App Chrome

**File:** `src/lib/components/` (MODIFY existing layout/header component)

Add a gear icon button to the app header that toggles the settings panel visibility. Store open/closed state in `uiStore`.

### F3. Budget Ceiling Visual Feedback

**File:** `src/lib/components/ui/TokenBudgetBar.svelte` (MODIFY)

Enhance the ceiling marker:
- Show soft/medium/hard threshold markers (derived from ceiling) as faint dotted lines
- Tooltip on ceiling marker showing exact percentage and derived thresholds
- When budget usage crosses a threshold, highlight that threshold marker briefly

### F4. Integration Tests

**File:** `src-tauri/tests/tool_lifecycle_integration.rs` (NEW)

End-to-end integration tests that exercise the full pipeline:

```rust
#[test]
fn anthropic_full_lifecycle() {
    // 1. Create engine, ingest 10+ turns to hit budget pressure
    // 2. Run rewriter → verify manifest injected, tools NOT injected (passive for Anthropic)
    // 3. Verify heuristic mutations applied (stalest blocks archived)
    // 4. Parse rewritten body → verify archived turns removed
}

#[test]
fn codex_chat_tool_injection() {
    // 1. Create engine with mature context (>3 non-system blocks)
    // 2. Build non-streaming ChatCompletions request
    // 3. Run rewriter → verify tools[] contains aperture_context_* definitions
    // 4. Verify manifest in system message
}

#[test]
fn codex_responses_tool_injection() {
    // Same as above but for Responses API format
}

#[test]
fn streaming_request_no_tools() {
    // 1. Build request with stream: true
    // 2. Run rewriter → verify tools NOT injected
    // 3. Verify manifest still injected
}

#[test]
fn first_turn_no_tools() {
    // 1. Create engine with 1 turn (small context)
    // 2. Run rewriter → verify no tools, no manifest
}

#[test]
fn context_tool_interception_dispatch() {
    // 1. Build mock response with aperture_context_preview call
    // 2. Call try_context_tool_interception
    // 3. Verify dispatch_tool called, results returned
}

#[test]
fn budget_pressure_archival_round_trip() {
    // 1. Create engine, fill to 85% budget
    // 2. Run rewriter → verify heuristic archival mutations
    // 3. Parse rewritten body → oldest middle-zone blocks removed
    // 4. Verify engine block updates applied (zone shifts, etc.)
}

#[test]
fn file_mutation_propagation_round_trip() {
    // 1. Ingest turns including a file read block for "auth.rs"
    // 2. Ingest a subsequent turn with edit_file("auth.rs") tool call
    // 3. Run rewriter → verify file_tracker detects mutation
    // 4. Verify content replacement in payload for the read block
}
```

~8 integration tests.

### F5. Frontend Tests

- Budget ceiling slider renders with correct initial value
- Slider change calls setBudgetCeiling with clamped value
- Derived thresholds display correctly
- TokenBudgetBar: threshold markers render at correct positions
- ContextBlock: compression badge shows for non-original blocks
- Context store: notifyBlockMutations fires toast on archival

~6 frontend tests.

### F6. Manual Test Playbook

**File:** `dev/active/metacog-dynamic-shifting/test-playbook.md` (NEW)

Structured manual test scenarios:

```markdown
# Phase 3 Manual Test Playbook

## Prerequisites
- Aperture built and running (`make dev`)
- At least one LLM client configured through proxy

## Test 1: Manifest Injection (Passive)
1. Start any client through proxy
2. Send a message and get a response
3. Send another message
4. Check proxy DEBUG logs for "Context rewriting applied"
5. Verify system message contains budget status line
Expected: Status line like "Context: 12% (24k/200k) | 6 blocks | calm"

## Test 2: Codex Tool Injection
1. Start Codex through proxy: OPENAI_BASE_URL=http://localhost:5400/v1 codex
2. Have 3+ exchanges to build context
3. Check proxy logs for tool injection
4. Verify request JSON contains aperture_context_* in tools[]
Expected: 5 context tools appended to tools array

## Test 3: Context Tool Interception
1. Continue Codex session from Test 2
2. Ask the model: "Before starting the next task, review your context window"
3. Watch proxy logs for:
   - "Extracted N context tool calls"
   - "Dispatching context tool: aperture_context_preview"
   - "Re-invoking with tool results"
4. Model should describe its context contents
Expected: Model successfully uses context tools without client seeing them

## Test 4: Budget Pressure Heuristics
1. Have a long Codex session (10+ exchanges)
2. Open Aperture UI, observe budget bar
3. If budget > 60%, check for archival markers on old blocks
4. Verify archived blocks show dissolve animation
5. On next exchange, verify archived blocks absent from request
Expected: Stale middle-zone blocks archived when budget pressured

## Test 5: Budget Ceiling Settings
1. Open settings panel (gear icon)
2. Adjust budget ceiling slider from 80% to 60%
3. Verify ceiling marker moves on budget bar
4. Verify soft/medium/hard threshold markers update
5. Close and reopen app — verify setting persists
Expected: Ceiling persists in localStorage and syncs to engine

## Test 6: File Mutation Tracking
1. In Codex session, ask model to read a file
2. Then ask model to edit that same file
3. On next exchange, check proxy logs for file mutation detection
4. Verify the old read block's content is updated in payload
Expected: Stale file content replaced with current version

## Test 7: Ephemeral Cleanup
1. After model uses context tools (Test 3)
2. On the NEXT request, check the conversation history
3. Context tool calls from previous turn should be stripped
4. A breadcrumb message should appear instead
Expected: Clean history with breadcrumb, no context tool clutter

## Test 8: Streaming Graceful Degradation
1. If client supports streaming, trigger a streaming request
2. Verify manifest is still injected
3. Verify tools are NOT injected (proxy logs should note streaming skip)
4. Heuristics should still apply
Expected: Full autonomous support, no interactive tools on streaming
```

### F7. Checkpoint F Tests Summary

| File | Tests | Focus |
|------|-------|-------|
| tool_lifecycle_integration.rs | ~8 | End-to-end pipeline |
| Frontend tests | ~6 | Settings UI, budget bar, blocks |
| **Total** | **~14** | |

---

## Checkpoint G: MCP Server for Claude Code

### G1. Architecture Decision

The design doc (Open Question #7) suggests: "local HTTP to Tauri backend is the likely answer."

**Approach:** Standalone Rust binary (`aperture-mcp`) that:
- Communicates with Claude Code over stdio (MCP protocol: JSON-RPC over stdin/stdout)
- Communicates with the Aperture engine over local HTTP (to the Tauri proxy port)
- Translates MCP tool calls into HTTP requests to engine endpoints

This avoids tight coupling to the Tauri process and works even if the MCP server is started independently.

### G2. Engine HTTP API for MCP

**File:** `src-tauri/src/proxy/handler.rs` (MODIFY)

Add internal API routes for context tool operations:

```
POST /_aperture/context/preview    → dispatch context_preview
POST /_aperture/context/read       → dispatch context_read
POST /_aperture/context/search     → dispatch context_search
POST /_aperture/context/plan       → dispatch context_plan
POST /_aperture/context/status     → dispatch context_status
GET  /_aperture/health             → health check
```

These routes are served by the same proxy axum server but routed to engine operations instead of upstream. The handler checks for `/_aperture/` prefix before normal proxy routing.

### G3. MCP Server Binary

**File:** `src-tauri/src/bin/aperture_mcp.rs` (NEW)

Standalone binary that:
1. Reads JSON-RPC messages from stdin
2. Responds to MCP protocol messages:
   - `initialize` → capabilities (tools)
   - `tools/list` → 5 context tools from `context_tool_definitions()`
   - `tools/call` → HTTP POST to `http://localhost:{port}/_aperture/context/{tool_name}`
3. Writes JSON-RPC responses to stdout

### G4. MCP Configuration for Claude Code

Users configure in their Claude Code MCP settings:

```json
{
  "mcpServers": {
    "aperture": {
      "command": "aperture-mcp",
      "args": [],
      "env": {
        "APERTURE_PORT": "5400"
      }
    }
  }
}
```

Or Aperture could auto-configure this when launching Claude Code via `aperture claude`.

### G5. ClaudeMcpRuntime Updates

**File:** `src-tauri/src/metacog/claude_mcp.rs` (MODIFY)

Currently `inject_tools()` and `extract_context_calls()` are no-ops (MCP handles tools natively). Update:
- `cleanup_history()` should strip MCP tool_use/tool_result blocks that match context tool names
- `inject_manifest()` should still inject into the Anthropic system message
- The runtime correctly returns empty from inject_tools/extract_context_calls (Claude's MCP transport handles these)

### G6. Engine API Route Handler

**File:** `src-tauri/src/proxy/context_api.rs` (NEW)

```rust
use axum::{Json, extract::State};

pub async fn handle_context_preview(
    State(state): State<Arc<ProxyState>>,
) -> Json<Value> {
    let engine = state.engine.as_ref().expect("engine required");
    let blocks = engine.active_session_blocks();
    let result = metacog::tools::dispatch_tool(
        "aperture_context_preview", &json!({}), &blocks, engine
    );
    Json(json!({ "content": result.content, "is_error": result.is_error }))
}

// Similar handlers for read, search, plan, status
```

### G7. Tests

| File | Tests | Focus |
|------|-------|-------|
| context_api.rs | ~5 | HTTP API endpoints |
| bin/aperture_mcp.rs | ~5 | MCP protocol handling |
| claude_mcp.rs | ~3 | Cleanup of MCP tool blocks |
| **Total** | **~13** | |

### G8. Manual Test: MCP Tool Lifecycle

```bash
# 1. Build the MCP server binary
cargo build --bin aperture-mcp

# 2. Start Aperture
make dev

# 3. Configure Claude Code MCP (add to project .mcp.json)

# 4. Launch Claude Code through proxy
aperture claude

# 5. Verify Claude discovers context tools (check /mcp output)
# 6. Ask Claude to "check your context window"
# 7. Verify it calls aperture_context_preview via MCP
# 8. Verify preview returns block inventory
# 9. Ask it to plan archival of old blocks
# 10. Verify context_plan called, mutations applied on next turn
```

---

## Critical Files Summary

| File | Checkpoint | Action | Purpose |
|------|-----------|--------|---------|
| `src-tauri/src/proxy/parser.rs` | E | MODIFY | Add stream field |
| `src-tauri/src/proxy/rewriter.rs` | E | MODIFY | Tool injection + first-turn gating |
| `src-tauri/src/proxy/handler.rs` | E, G | MODIFY | Response interception + API routes |
| `src-tauri/src/proxy/interceptor.rs` | E | NEW | Tool interception + re-invoke loop |
| `src-tauri/src/engine/mod.rs` | E | MODIFY | Internal mutation methods |
| `src-tauri/src/proxy/mod.rs` | E, G | MODIFY | Module registration |
| `src/lib/components/settings/SettingsPanel.svelte` | F | NEW | Budget ceiling + stats |
| `src/lib/components/ui/TokenBudgetBar.svelte` | F | MODIFY | Threshold markers |
| `src-tauri/tests/tool_lifecycle_integration.rs` | F | NEW | Integration tests |
| `dev/active/metacog-dynamic-shifting/test-playbook.md` | F | NEW | Manual test procedures |
| `src-tauri/src/proxy/context_api.rs` | G | NEW | Engine HTTP API |
| `src-tauri/src/bin/aperture_mcp.rs` | G | NEW | MCP server binary |
| `src-tauri/src/metacog/claude_mcp.rs` | G | MODIFY | MCP cleanup |

## Implementation Order

1. **Checkpoint E** (largest — core lifecycle):
   E1 → E2 → E3 → E6 → E5 → E4 → E7 → E8 tests → E9 manual test

2. **Checkpoint F** (polish + verification):
   F1 → F2 → F3 → F4 → F5 → F6 → F7 all tests green

3. **Checkpoint G** (MCP server — last):
   G2 → G6 → G3 → G5 → G4 → G7 tests → G8 manual test

## Verification Per Checkpoint

```bash
# After each checkpoint:
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml
npm run check
npx vitest run
make check  # all green
```

## Test Count Projections

| Checkpoint | New Rust Tests | New Frontend Tests | Running Total |
|-----------|---------------|-------------------|---------------|
| Current | 421 | 37 | 458 |
| E | ~24 | 0 | ~482 |
| F | ~8 | ~6 | ~496 |
| G | ~13 | 0 | ~509 |
