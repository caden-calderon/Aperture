# Phase 3 Manual Test Playbook

> Structured manual test scenarios for validating the metacognition + dynamic context shifting pipeline.

## Prerequisites

- Aperture built and running (`make dev`)
- At least one LLM client configured through proxy (port 5400)
- Proxy logs visible in terminal (set `RUST_LOG=info,aperture_lib=debug`)

---

## Test 1: Manifest Injection (Passive/ClaudeMcp)

**Goal:** Verify the context status line is injected into system messages.

1. Start Aperture: `make dev`
2. Start any client through proxy (e.g., `aperture claude` or `OPENAI_BASE_URL=http://localhost:5400/v1 codex`)
3. Send a message and get a response
4. Send another message (need at least 1 non-system block for manifest gating)
5. Check proxy DEBUG logs for `"Injected manifest into system message"`
6. If using Codex, the system/instructions field should contain a status line like:
   ```
   Context: 12% (24k/200k) | 6 blocks | calm
   ```

**Expected:** Status line appears in logs after the second exchange. First-turn gating means no manifest on the very first empty context.

---

## Test 2: Codex Tool Injection (Non-Streaming)

**Goal:** Verify aperture_context_* tools are injected into non-streaming Codex requests.

1. Start Codex through proxy: `OPENAI_BASE_URL=http://localhost:5400/v1 codex`
2. Have 3+ exchanges to build up context (>3 non-system blocks)
3. Check proxy DEBUG logs for `"Injected context tools into request"`
4. Inspect the forwarded request JSON — the `tools` array should contain 5 aperture tools:
   - `aperture_context_preview`
   - `aperture_context_read`
   - `aperture_context_search`
   - `aperture_context_plan`
   - `aperture_context_status`

**Expected:** Tools appear in request after context reaches maturity threshold (>3 non-system blocks). On the first 1-2 turns, tools should NOT be injected.

---

## Test 3: Context Tool Interception

**Goal:** Verify the proxy intercepts and dispatches context tool calls transparently.

1. Continue Codex session from Test 2 (mature context)
2. Ask the model: "Before starting the next task, review your context window"
3. Watch proxy logs for:
   - `"Extracted N context tool calls"`
   - `"Dispatching context tool: aperture_context_preview"`
   - `"Re-invoking with tool results"` (if context-only response)
4. The model should describe its context contents naturally

**Expected:** Model successfully uses context tools. The client never sees the context tool calls — they're handled transparently by the proxy. The model gets results and continues.

---

## Test 4: Budget Pressure Heuristics

**Goal:** Verify automatic archival of stale blocks under budget pressure.

1. Have a long Codex session (10+ exchanges) to build significant context
2. Open Aperture UI, observe the budget bar
3. If budget utilization > 60%, check for:
   - Proxy logs: `"Applied rewrite decisions: N turns removed"`
   - UI: Archived blocks show dissolve animation
4. On the next exchange, verify archived blocks are absent from the request payload
5. Check that Primacy-zone (system) and Recency-zone (recent) blocks are preserved

**Expected:** Stale middle-zone blocks are archived when budget pressure triggers. System prompt and recent turns are always protected.

---

## Test 5: Budget Ceiling Settings

**Goal:** Verify the settings UI controls the budget ceiling and persists state.

1. Open settings panel (gear icon in title bar)
2. Observe the current ceiling value (default: 80%)
3. Adjust budget ceiling slider from 80% to 60%
4. Verify:
   - Ceiling marker moves on the budget bar
   - Soft/medium/hard threshold markers update (derived from ceiling)
   - Threshold labels in settings update
5. Close settings, reopen — verify value persists
6. Close and reopen the entire app — verify value persists (localStorage)
7. Set ceiling to 40% (minimum) — verify slider stops
8. Set ceiling to 100% (maximum) — verify slider stops

**Expected:** Ceiling persists in localStorage (`aperture:budget-ceiling`) and syncs to engine via IPC. Derived thresholds (soft = 75% of ceiling, medium = 88%, hard = 95%) update reactively.

---

## Test 6: File Mutation Tracking

**Goal:** Verify stale file content is updated in context when files are edited.

1. In Codex session, ask the model to read a file (e.g., "Read src/main.rs")
2. Wait for the file read to complete (tool_result block created)
3. Then ask the model to edit that same file (e.g., "Add a comment at the top of src/main.rs")
4. On the next request, check proxy logs for:
   - `"Applied file mutation tracking"` or related file tracker debug output
5. Verify the old read block's content is updated in the payload (not stale)

**Expected:** File tracker detects the edit_file call, finds blocks referencing the same file, and generates UpdateContent mutations. The rewriter replaces stale file content in the outgoing payload.

---

## Test 7: Ephemeral Cleanup

**Goal:** Verify context tool calls are stripped from history on subsequent turns.

1. After model uses context tools (Test 3), continue the conversation
2. On the NEXT request, check the outgoing conversation history in proxy logs
3. Context tool calls from the previous turn should be stripped
4. A breadcrumb message should appear instead (e.g., "[Aperture managed context: archived 2 blocks]")

**Expected:** Clean history with breadcrumb replacing ephemeral tool calls. No `aperture_context_*` tool_use/tool_result blocks in the forwarded history.

---

## Test 8: Streaming Graceful Degradation

**Goal:** Verify streaming requests get autonomous support but no interactive tools.

1. Trigger a streaming request (most Codex requests are streaming by default for response generation)
2. Check proxy logs for:
   - `"No payload rewriting needed"` OR manifest injection log (but NOT tool injection)
   - Streaming detection: `parsed.stream = true`
3. Verify manifest IS still injected (status line in system message)
4. Verify tools are NOT injected
5. Heuristic archival should still apply to the request payload

**Expected:** Full autonomous support (manifest + heuristics) on streaming requests. No interactive tools injected. Response streams through to client unmodified.

---

## Verification Checklist

After completing all tests, verify:

- [ ] `make check` passes (cargo clippy, cargo fmt, cargo test, svelte-check, vitest)
- [ ] No regressions in existing proxy flow tests
- [ ] UI animations render correctly (block materialization, archived dissolve, compressed dashed border)
- [ ] Budget bar updates reflect real token counts
- [ ] Settings panel opens/closes cleanly
- [ ] No console errors in WebKitGTK devtools
