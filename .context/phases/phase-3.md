# Phase 3: Metacognition + Dynamic Context Shifting

**Status**: IMPLEMENTATION COMPLETE (A-G) — staff review complete, remediation pending before Phase 4.
**Goal**: Give the model awareness and control of its own context, and build an autonomous system that continuously optimizes the context window
**Prerequisites**: Phase 2 complete
**Design Doc**: `dev/active/metacog-dynamic-shifting/design.md`
**Review + Remediation Docs**:
- `dev/active/metacog-dynamic-shifting/staff-review-2026-02-13.md`
- `dev/active/metacog-dynamic-shifting/plan.md`
- `dev/active/metacog-dynamic-shifting/tasks.md`

---

## Context from Phase 2

Phase 2 delivers:
- Context engine with block management and zone system (primacy/middle/recency)
- Accurate token counting via tiktoken
- Staleness scoring for blocks
- Classification pipeline with auto-assignment
- Multi-session management
- Deterministic dependency tracking (file references)
- Thread-line grouping (structural constraints)
- Proxy with full traffic capture, SSE tee, hot-patch

**Key insight**: The proxy sees every message, every tool call, every response. Since all major LLM tools are stateless (send full conversation each request), the proxy can rewrite the entire payload on every turn — reorder, compress, inject, remove.

---

## Problem Statement

1. **Models are blind** — No awareness of their own context state, budget, or what's been compressed/archived
2. **Context is append-only** — Blocks accumulate, nothing is actively managed
3. **Catastrophic compression** — Current paradigm: hit wall → compress everything → lose info
4. **No predictive management** — System doesn't anticipate what context the next task needs
5. **Stale code in context** — File reads persist in history even after the model edits those files
6. **No collaboration** — The model can't participate in managing its own memory

---

## Deliverables

### 1. Context Management Tools + Client Adapter Layer

Context tools exposed to models through a `ContextToolRuntime` trait with client-specific adapters:

- **ClaudeMcpRuntime**: Tools via MCP server. Claude Code discovers and calls natively. Primary path.
- **CodexProxyRuntime**: Tools injected into API request `tools[]` by proxy. Proxy intercepts and handles. Same capabilities, slightly more latency.
- **PassiveRuntime**: Manifest only, no tools. System heuristics handle everything. Fallback for unknown clients.

Shared core (planner, manifest, cleanup, heuristics, block store, mutations) is identical across all runtimes.

```
aperture_context_preview()              → Block inventory with smart-extracted previews
aperture_context_read(block_id)         → Full content of a block
aperture_context_search(query, scope?)  → Search active + archived blocks
aperture_context_plan(actions)          → Plan changes (incl. model-authored compressions + splits), return preview
aperture_context_status()               → Full detailed manifest on demand
```

Two-tier viewing: `preview()` shows smart extractions (function names, file paths, code snippets, key text) for all blocks. `read()` shows full content. No middle tier.

**Plan → Preview → Apply pattern**: Tools are read-only exploration + planning. No side effects during the turn. The proxy applies mutations between turns. Last `context_plan()` call wins — model can iterate on its plan.

**Client-agnostic**: Same tools available via MCP (Claude), proxy injection (Codex), or degraded to passive mode (unknown clients).

### 2. Context Planner Module

The brain that ties everything together. Runs between turns:

```
Inputs:
  - Current engine state (blocks, zones, staleness, tokens)
  - Planned changes from context_plan (if any)
  - System heuristics (budget pressure, staleness, task detection)
  - File mutation tracking (edits to files with blocks in context)

Outputs:
  - Mutations to apply (compress, archive, recall, reorder, update)
  - Manifest to inject (status line + delta + warnings)
  - Cleanup instructions (strip ephemeral tool calls, insert breadcrumb)
  - Updated block positions (primacy/middle/recency ordering)
```

### 3. Layered Awareness (Manifest System)

Tiered manifest that scales with need:

- **Always** (~30 tok): Status line in primacy — budget %, block counts, pending actions
- **On change** (~50-100 tok): Delta showing what shifted since last turn
- **On demand** (~200-300 tok): Full inventory via `context_status()` tool
- **On pressure** (~100-150 tok): Budget warning with recommendations

### 4. Ephemeral Tool Call Cleanup

Between turns:
1. Strip all `aperture_context_*` tool_use entries from conversation history
2. Strip all corresponding tool_result entries
3. Replace with breadcrumb summary (~50 tok)
4. Apply planned mutations to engine state
5. Update manifest

The model wakes up with clean history, context in the right state, and a breadcrumb of what changed. Zero memory of the exploration process.

### 5. Autonomous Heuristics

Always-running system-driven context management:

- **Budget pressure**: Progressive compression as capacity grows (soft/medium/hard thresholds)
- **Staleness decay**: Blocks not referenced in N turns get progressively compressed
- **Task detection**: File reference shifts signal task boundaries, trigger relevance re-scoring
- **Dependency overlap**: Current task touches file X → boost relevance of blocks referencing X
- **File mutation tracking**: Edit operations update corresponding read blocks in context

### 6. Dynamic Code Context Updates

When the proxy sees a file edit (via tool call results), it updates all blocks containing reads of that file — active AND archived. The model never sees stale file content.

### 7. Model-Authored Compression

The model writes its own summaries during context management via `context_plan()`. It knows what matters, what the next task needs, and what details to preserve. Compression text is included in the plan and applied on turn end.

For autonomous/bulk compression (system-driven, many blocks), defers to Phase 4's sidekick LLM. Phase 3 uses archival (remove from payload) as the default autonomous action.

### 8. Smart Preview Extraction

Rule-based extraction of identifying elements from blocks: function/class names, file paths, code snippets, key phrases. Not a summary — a table of contents. Gives enough signal for the model to decide "do I need this?" without seeing full content.

### 9. Configurable Budget Ceiling

User sets max comfortable context usage. Three internal thresholds derived:
- **Soft** (~50% of ceiling): Start archiving stalest blocks
- **Medium** (~80% of ceiling): Archive middle zone aggressively
- **Hard** (ceiling): Emergency archival, only primacy + current task remain

### 10. Trigger System

Multiple trigger sources for context management:
- **Self-directed**: Model decides on its own
- **Task completion**: System detects task boundary, nudges model
- **Budget warnings**: System injects warning at thresholds
- **Configurable**: User controls when/how triggers fire

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/engine/planner.rs` | **NEW** | Context planner (signal collection, mutation planning) |
| `src-tauri/src/engine/planner/manifest.rs` | **NEW** | Manifest generation (layered) |
| `src-tauri/src/engine/planner/heuristics.rs` | **NEW** | Autonomous heuristics (budget, staleness, task detection) |
| `src-tauri/src/engine/planner/cleanup.rs` | **NEW** | Ephemeral tool call stripping + breadcrumbs |
| `src-tauri/src/engine/planner/relevance.rs` | **NEW** | Relevance scoring (recency + deps + signals) |
| `src-tauri/src/engine/planner/file_tracker.rs` | **NEW** | File mutation tracking across tool calls |
| `src-tauri/src/metacog/runtime.rs` | **NEW** | `ContextToolRuntime` trait + shared types |
| `src-tauri/src/metacog/claude_mcp.rs` | **NEW** | ClaudeMcpRuntime (MCP server adapter) |
| `src-tauri/src/metacog/codex_proxy.rs` | **NEW** | CodexProxyRuntime (proxy-injected tools) |
| `src-tauri/src/metacog/passive.rs` | **NEW** | PassiveRuntime (manifest only, heuristics-driven) |
| `src-tauri/src/metacog/tools.rs` | **NEW** | Context tool implementations (shared across runtimes) |
| `src-tauri/src/metacog/mod.rs` | **NEW** | Module root, runtime selection |
| `src-tauri/src/proxy/handler.rs` | Modify | Planner integration between turns, cleanup application |
| `src-tauri/src/engine/mod.rs` | Modify | Planner wiring, file tracking hooks |
| `src-tauri/src/engine/block.rs` | Modify | Block state for compression/archive tracking |
| `src/lib/components/blocks/ContextBlock.svelte` | Modify | Visual state transitions (compress/archive animations) |
| `src/lib/stores/context.svelte.ts` | Modify | Planner-driven state updates |

---

## context_plan() Actions Schema

The `context_plan(actions)` tool accepts a JSON object with the following optional fields:

```json
{
  "expand": [8, 14],                    // Block IDs to expand to full content
  "archive": [12, 3],                   // Block IDs to remove from payload (kept in storage)
  "recall": [21],                       // Archived block IDs to bring back into active context
  "pin": [8],                           // Block IDs to pin (prevent auto-archival)
  "unpin": [5],                         // Block IDs to unpin
  "shift_to": { "8": "primacy" },       // Block ID → target zone
  "compress": {                         // Block ID → model-authored summary text
    "15": "CSS styles for context blocks. Tailwind @apply for layout."
  },
  "split": {                            // Thread ID → split instructions
    "thread_23": { "at": 5, "archive_before": true }
  }
}
```

All fields optional. Last `context_plan()` call in a turn replaces the previous entirely.

---

## Implementation Steps

This phase is structured into **4 checkpoint sessions**. Each checkpoint is a natural pause point — run `make check`, verify tests pass, update RESUME.md, and clear context before continuing.

---

### Checkpoint A: Core Foundation (Steps 1-2) ✅ COMPLETE (2026-02-13)
> **Goal**: Planner types + all 5 tools working against mocked engine state. No runtime adapters yet.
> **Exit criteria**: `make check` passes, 22+ unit tests (planner 14 + tools 8), manifest generates correctly.
> **Result**: 69 new tests (38 planner + 31 metacog), 260 total passing, all checks clean.

#### Step 1: Context Planner Foundation (~12k context)

1. Create `engine/planner/` module structure (mod.rs, manifest.rs, types.rs)
2. Define planner input/output types:
   - `PlannerInput`: engine state snapshot, pending context_plan actions, heuristic signals
   - `PlannerOutput`: mutations to apply, manifest to inject, cleanup instructions
   - `ContextMutation` enum: Expand, Archive, Recall, Pin, Unpin, Shift, Compress, Split
3. Implement manifest generation:
   - `ManifestLevel::StatusLine` (~30 tok): budget %, block counts
   - `ManifestLevel::Delta` (~50-100 tok): what changed since last turn
   - `ManifestLevel::Full` (~200-300 tok): complete block inventory with previews
   - `ManifestLevel::Warning` (~100-150 tok): budget pressure with recommendations
4. Wire planner struct into engine (planner holds reference to engine state, callable between turns)
5. Unit tests: manifest at each level, planner input/output round-trip, mutation types

#### Step 2: Context Tool Implementations (~12k context)

1. Define `ContextToolRuntime` trait in `metacog/runtime.rs`:
   - `register_tools()`, `extract_context_calls()`, `inject_results()`, `cleanup_history()`, `inject_manifest()`
2. Implement shared tool logic in `metacog/tools.rs`:
   - `context_preview()` — iterate engine blocks, generate smart-extracted previews grouped by zone
   - `context_read(id)` — fetch full block content from engine by ID
   - `context_search(query, scope)` — keyword search across active blocks + optionally archived. Match against content, file paths, role
   - `context_plan(actions)` — validate actions against current state, compute preview (token deltas, new zone layout). Store as pending plan. Last-plan-wins.
   - `context_status()` — generate full manifest (delegates to planner manifest at Full level)
3. Implement smart preview extraction in `metacog/preview.rs`:
   - Regex-based extraction: function/class/struct names, file paths, import statements
   - First/last N lines for non-code blocks
   - Code blocks: signature lines + key identifiers
4. Unit tests: each tool against mocked engine state, preview extraction on various block types

---

### Checkpoint B: Client Adapters + Cleanup (Steps 3-4) ✅ COMPLETE (2026-02-13)
> **Goal**: Tools work through all 3 runtimes. Ephemeral cleanup strips tool calls and leaves breadcrumbs. First end-to-end turn cycle works.
> **Exit criteria**: `make check` passes, 38+ unit tests, breadcrumb generation verified, cleanup handles mixed real + context tools.
> **Result**: 59 new tests (22 cleanup + 8 claude_mcp + 14 codex_proxy + 7 passive + 5 mod + 2 planner + 1 runtime), 338 total passing, all checks clean.

#### Step 3: Client Adapters (~10k context)

1. Implement `ClaudeMcpRuntime` in `metacog/claude_mcp.rs`:
   - MCP server registration (stdio transport)
   - Tool definitions matching MCP schema
   - IPC to engine (local HTTP to Tauri backend — validate this works first as a spike)
   - Result formatting for MCP tool_result
2. Implement `CodexProxyRuntime` in `metacog/codex_proxy.rs`:
   - Inject tool definitions into OpenAI-format `tools[]` array on outgoing requests
   - Detect context tool calls in response `tool_calls[]` (match by function name prefix `aperture_context_`)
   - Separate context calls from real calls, handle context calls internally
   - Inject context tool results into next request's messages alongside real tool results
   - Handle only-context-calls case (re-invoke API with results)
3. Implement `PassiveRuntime` in `metacog/passive.rs`:
   - `register_tools()` returns empty vec
   - `extract_context_calls()` returns empty vec
   - `inject_manifest()` injects status line into system message
   - No tool surface — all management via heuristics
4. Implement runtime selection in `metacog/mod.rs`:
   - Detect client from: Anthropic API paths → Claude, OpenAI API paths → Codex, unknown → Passive
   - Configurable override in settings
5. Unit tests: each adapter with mock tool calls, runtime selection logic

#### Step 4: Ephemeral Cleanup System (~10k context)

1. Implement tool call detection in `engine/planner/cleanup.rs`:
   - Identify `aperture_context_*` tool_use blocks in conversation history
   - Handle both Anthropic format (tool_use content blocks) and OpenAI format (tool_calls array)
2. Implement stripping logic:
   - Remove matched tool_use entries
   - Remove corresponding tool_result entries (match by tool_use_id)
   - Preserve all non-context tool calls and all text content
3. Implement breadcrumb generation:
   - Summarize applied mutations in one line (~50 tok)
   - Format: `[Context update: expanded #8 → primacy, archived #12. Net: -1,960 tok. Budget: 52%]`
   - Insert as a system message or annotation in place of stripped calls
4. Wire cleanup into proxy handler:
   - On incoming request: run cleanup on messages array before forwarding
   - Apply pending mutations from last turn's context_plan
   - Inject manifest (status line + delta if changes occurred)
5. Unit tests: cleanup with mixed real + context tools, cleanup with no context tools, breadcrumb formatting, per-runtime message format handling

---

### Checkpoint C: Autonomous Intelligence (Steps 5-6) ✅ COMPLETE (2026-02-13)
> **Goal**: System manages context without model involvement. File edits propagate. The planner runs heuristics alongside model-planned changes.
> **Exit criteria**: `make check` passes, 49+ unit tests, heuristics trigger archival at budget thresholds, file edits update read blocks.
> **Result**: 53 new tests (16 heuristics + 14 relevance + 19 file_tracker + 4 integration), 371 total passing, all checks clean.

#### Step 5: Autonomous Heuristics (~12k context)

1. Implement budget pressure in `engine/planner/heuristics.rs`:
   - Read user's budget ceiling config (default: 80%)
   - Derive soft/medium/hard thresholds
   - At soft: flag stalest blocks for archival
   - At medium: generate archival mutations for all middle-zone stale blocks
   - At hard: aggressive archival, protect only primacy + recency
2. Implement staleness-driven archival:
   - Blocks not referenced in N turns (configurable, default: 10) → candidate for archival
   - Staleness score from Phase 2 engine feeds into archival priority
3. Implement task detection:
   - Track file references per turn (from tool calls: read_file, edit_file, etc.)
   - Detect significant shift (>50% new files vs previous turn) → signal task boundary
   - On task boundary: re-score all blocks for relevance to new file set
4. Implement dependency-based relevance boosting in `engine/planner/relevance.rs`:
   - Current turn touches file X → find all blocks that reference file X → boost relevance
   - Boosted blocks resist archival, may shift toward recency
5. Wire heuristics into planner:
   - Planner runs heuristics AFTER applying model's planned changes
   - Conflict resolution: model intent (pins, explicit plans) overrides heuristics
   - Heuristic mutations added to planner output
6. Unit tests: budget thresholds trigger correct archival counts, staleness scoring, task detection on file reference shifts, relevance boosting, conflict resolution (model pin vs heuristic archival)

#### Step 6: File Mutation Tracking (~8k context)

1. Implement file tracker in `engine/planner/file_tracker.rs`:
   - Parse tool call names from proxy traffic (match `write_file`, `edit_file`, `Write`, `Edit`, etc.)
   - Extract file path from tool call arguments
   - Extract new content or diff from tool result
2. Map edits to blocks:
   - Find blocks whose content originated from a read of the same file path
   - Apply edit (replace content with new version, or apply diff)
3. Handle archived blocks:
   - Same mapping applies — archived reads of the same file get updated
   - When recalled, model gets current version, not stale version
4. Edge cases:
   - File deleted → mark corresponding blocks as stale/orphaned
   - File renamed → track if rename tool call provides old→new path
   - Multiple reads of same file → update all of them
5. Unit tests: edit detection for various tool name patterns, block content update, archived block update, multiple reads of same file

---

### Checkpoint D: Integration + UI (Steps 7-8)
> **Goal**: Full end-to-end working system. Payload rewriting positions blocks correctly. UI shows everything in real-time. Live-validated with Claude Code and Codex.
> **Exit criteria**: `make check` passes, 55+ unit tests, 12+ integration tests, live validation with both clients.

#### Step 7: Payload Rewriting + Position Management (~10k context)

1. Implement block positioning in proxy handler:
   - Sort blocks by zone: primacy first, middle in staleness order, recency last
   - Within zones, maintain thread ordering (turn-based)
   - Pinned blocks in their assigned zone
2. Implement thread-group atomic reordering:
   - Use Phase 2 thread grouping utility to identify atomic units
   - Move entire thread groups, never split (unless model directed via context_plan split)
   - Validate: no tool_call orphaned from its tool_result after reordering
3. Implement manifest injection:
   - Insert status line as first system message content (primacy)
   - Insert delta/warning if applicable
   - Insert breadcrumb from cleanup
4. Integration tests:
   - Full turn cycle through ClaudeMcpRuntime: request → tool calls → response → cleanup → next request payload verification
   - Full turn cycle through CodexProxyRuntime: same flow with proxy-injected tools
   - Full turn cycle through PassiveRuntime: manifest injection + heuristics-only
   - Budget pressure scenario: fill context → verify heuristic archival → verify manifest warning
   - File mutation: edit → verify read block updated in next payload

#### Step 8: UI Integration (~8k context)

1. Block state transition animations:
   - Compress: block shrinks with dither/dissolve effect (Obra Dinn aesthetic)
   - Archive: fade out + slide off
   - Recall: fade in from side
   - Shift: blocks physically slide to new zone position
2. Token budget bar:
   - Live display showing current usage vs ceiling
   - Animate changes (grows on new content, shrinks on archival)
   - Color shifts at threshold boundaries (green → yellow → red)
3. Activity feed in status bar:
   - "Compressed 3 blocks (-1,200 tok)" / "Archived: test output" / "Model requested: expand auth.rs"
   - Brief, auto-dismiss after N seconds
4. Zone boundary visualization:
   - Primacy/middle/recency zones visually distinct
   - Blocks animate between zones on shift
   - Minimap updates in real-time
5. Budget ceiling setting:
   - Settings panel: slider or input for max context usage %
   - Visual indicator on budget bar showing ceiling position
6. Frontend tests for new components

---

## Test Coverage

### Unit Tests (~55 tests)

| File | Tests | Focus |
|------|-------|-------|
| `engine/planner.rs` | 8 | Planner input/output, mutation planning |
| `engine/planner/manifest.rs` | 6 | Status line, delta, full manifest generation |
| `engine/planner/heuristics.rs` | 8 | Budget thresholds, staleness triggers, task detection |
| `engine/planner/cleanup.rs` | 6 | Tool call stripping, breadcrumb generation |
| `engine/planner/relevance.rs` | 5 | Scoring: recency, deps, signals |
| `engine/planner/file_tracker.rs` | 4 | Edit detection, block update |
| `metacog/tools.rs` | 8 | Each tool with mocked engine state |
| `metacog/claude_mcp.rs` | 4 | MCP registration, result handling, cleanup |
| `metacog/codex_proxy.rs` | 4 | Tool injection, call interception, result injection |
| `metacog/passive.rs` | 2 | Manifest-only injection, no tool surface |

### Integration Tests (~12 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/planner_integration.rs` | 5 | Full turn cycle: request → response → cleanup → payload |
| `tests/metacog_claude.rs` | 2 | MCP tool → engine state → planner → payload |
| `tests/metacog_codex.rs` | 2 | Proxy-injected tool → intercept → engine → payload |
| `tests/metacog_passive.rs` | 1 | Manifest injection + heuristics-only path |
| `tests/file_tracking.rs` | 2 | Edit detection → block update → next payload reflects change |

### Manual/Live Tests

| Test | Description |
|------|-------------|
| `test_manifest_visible` | Model sees status line, can call context_status() |
| `test_tool_exploration` | Model calls preview → read → plan, gets real results |
| `test_cleanup_invisible` | After context management turn, next turn shows clean history + breadcrumb |
| `test_budget_pressure` | Fill context to threshold, verify warning injected, model responds |
| `test_autonomous_compression` | Long session, verify system auto-compresses stale blocks |
| `test_file_update` | Edit a file, verify read block updates in context |
| `test_side_by_side` | Same task with/without Aperture, compare outcomes |

---

## Success Criteria

- [ ] `ContextToolRuntime` trait implemented with three adapters (Claude MCP, Codex proxy, Passive)
- [ ] Claude MCP runtime: tools discoverable and callable from Claude Code
- [ ] Codex proxy runtime: tools injected and intercepted correctly
- [ ] Passive runtime: manifest injected, heuristics drive all management
- [ ] `context_preview()` returns block inventory with smart-extracted previews
- [ ] `context_search()` finds blocks across active + archived
- [ ] `context_plan()` accepts model-authored compressions and thread splits, returns accurate preview
- [ ] Last-plan-wins: multiple `context_plan()` calls replace, not accumulate
- [ ] Ephemeral cleanup strips context tool calls, leaves breadcrumb
- [ ] Manifest status line present in primacy every turn
- [ ] Delta injected only when context actually changed
- [ ] Budget warning injected at configurable thresholds
- [ ] Autonomous heuristics archive stale blocks without model intervention
- [ ] File edits propagate to corresponding read blocks (active + archived)
- [ ] Thread groups stay atomic during reordering (model-directed splits respected)
- [ ] Block positions reflect zone assignment in actual API payload
- [ ] UI shows real-time state transitions (compress, archive, recall animations)
- [ ] Budget ceiling configurable from UI
- [ ] `make check` passes
- [ ] 55+ unit tests passing
- [ ] 12+ integration tests passing
- [ ] Live-validated with Claude Code (MCP) and Codex CLI (proxy-injected) through proxy

---

## Key Imports for Next Phase

```rust
use crate::engine::planner::{
    ContextPlanner, PlannerInput, PlannerOutput,
    Manifest, ManifestLevel,
    Heuristics, BudgetPolicy,
    Cleanup, Breadcrumb,
};
use crate::metacog::{
    ContextToolRuntime, ContextTools,
    ClaudeMcpRuntime, CodexProxyRuntime, PassiveRuntime,
};
```

```typescript
// UI integration
import { contextPlannerState } from '$lib/stores/planner.svelte';
// Animations
import { compressTransition, archiveTransition, recallTransition } from '$lib/transitions';
```

---

## Relationship to Phase 4 (Compression)

Phase 3 builds the intelligence layer (what to compress, when, where to put it) and includes model-authored compression (the model writes its own summaries via `context_plan()`).

Phase 4 builds the automated mechanism — sidekick LLM integration for bulk/autonomous compression, compression queue, preserve-keys, and quality scoring.

Phase 3's primary token-saving mechanism is **archival** (remove from payload, keep in storage). Model-authored compression handles important blocks the model explicitly manages. Phase 4 adds the sidekick path so the system can autonomously compress without bothering the model.
