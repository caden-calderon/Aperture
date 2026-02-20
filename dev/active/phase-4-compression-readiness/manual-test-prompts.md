# Phase 4 Manual Test Prompts

## Setup
1. `make dev` — launch Aperture
2. In Aperture Settings (gear icon), set **Budget Ceiling** to **50%**
   - This makes: soft=25%, medium=40%, hard=50%
   - With 200k token context, warnings should fire around 50k tokens
3. `aperture claude` — launch Claude Code through proxy
4. Paste prompts below

---

## Prompt 1: Build Context + Verify Silence

Paste this as your first message to Claude Code through Aperture:

```
I need you to help me explore the Aperture codebase. Do these tasks one at a time:

1. Read these files and summarize what each does in 1 sentence:
   - src-tauri/src/proxy/rewriter.rs
   - src-tauri/src/engine/planner/mod.rs
   - src-tauri/src/engine/planner/heuristics.rs
   - src-tauri/src/proxy/handler.rs
   - src-tauri/src/metacog/runtime.rs
   - src-tauri/src/metacog/tools.rs

2. Read these files too:
   - src-tauri/src/engine/mod.rs
   - src-tauri/src/engine/budget.rs
   - src-tauri/src/proxy/parser.rs
   - src-tauri/src/proxy/interceptor.rs

3. After reading all files, write a brief summary (5-10 lines) of how the proxy-to-engine data flow works.

4. Then read:
   - src-tauri/tests/tool_lifecycle_integration.rs
   - src-tauri/src/engine/planner/types.rs
   - src-tauri/src/engine/planner/manifest.rs

5. Finally, check the test suite: run `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result:"` and report the counts.

IMPORTANT: After each step, pause and tell me what step you just completed.
```

### What to Observe (Phase 1)
- **Aperture UI**: Watch the token count climb in Settings → Context Stats
- **System message**: Should NEVER contain `[Aperture:` text (check proxy logs with `RUST_LOG=debug`)
- **No warnings injected**: Until utilization crosses 25% (soft threshold at 50% ceiling), Claude should see NO Aperture messages in its context
- **Tool calls work normally**: Claude reads files, runs commands — proxy is transparent
- **Heuristics silent**: No archival mutations happening between turns

---

## Prompt 2: Threshold Crossing + MCP Tools + Batch Behavior

After Prompt 1 has built up some context, paste this:

```
Now I want to test Aperture's context management capabilities. Do these in order:

1. Call the aperture_context_status tool to see the current budget status. Report what it says.

2. Call aperture_context_preview to see a summary of all blocks in context. How many blocks are there? What zones are they in?

3. Now let's build more context. Read these additional files:
   - src-tauri/src/proxy/context_api.rs
   - src-tauri/src/bin/aperture_mcp.rs
   - src-tauri/src/engine/planner/cleanup.rs
   - src-tauri/src/engine/planner/relevance.rs
   - src-tauri/src/engine/planner/file_tracker.rs
   - src-tauri/src/engine/planner/applicator.rs
   - src-tauri/src/metacog/claude_mcp.rs
   - src-tauri/src/metacog/codex_proxy.rs

4. After reading those, call aperture_context_status again. Has the utilization changed? Did you see any warning messages appear in the conversation?

5. Call aperture_context_search with query "cache" to find cache-related blocks.

6. Call aperture_context_plan with these actions:
   {"archive": [], "pin": [], "expand": [], "compress": []}
   (empty plan — just to test the tool works)

7. Report a summary:
   - Total blocks in context
   - Current utilization %
   - Did any [Aperture: ...] warning messages appear during this session?
   - Did any blocks get automatically archived?
   - Were the context tools responsive?
```

### What to Observe (Phase 2)
- **aperture_context_status works**: Returns budget info without manifest injection
- **Warning injection**: When utilization crosses 25% (soft at 50% ceiling), a ONE-TIME warning should appear in the last user message. Only once — not repeated.
- **No second warning**: Subsequent requests at the same pressure level should be silent
- **Tools responsive**: All 5 MCP tools should return valid responses
- **Batch gating**: Heuristic archival should only fire at batch points (task boundary between file reading groups, or threshold crossing), not on every request
- **System message untouched**: Throughout both prompts, the system message should never be modified

---

## What to Check After Tests

### In Aperture UI
- Context Stats shows accurate block count and utilization
- Budget bar reflects the 50% ceiling
- Threshold markers show soft=25%, medium=40%, hard=50%

### In Proxy Logs (RUST_LOG=debug)
Look for these log lines:
- `"Injected budget warning/breadcrumb into last user message (cache-safe)"` — should appear at most 1-3 times (threshold crossings only)
- `"No payload rewriting needed"` — should appear on most requests (proxy is transparent)
- `"Batch point: applying N heuristic mutations"` — only at threshold crossings or task boundaries
- `"No batch point this turn"` — most requests should hit this path
- NEVER: `"Injected manifest"` — this is the old behavior, should not appear

### Cost Validation
After the test session, check the API usage:
- `cache_read_input_tokens` should be significantly > 0 (cache is hitting)
- `cache_creation_input_tokens` should only spike on first request and threshold crossings
- Compare to the pre-fix sessions where cache_read was 0
