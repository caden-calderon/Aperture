# Aperture Resume Context

> **Read this file first when starting a fresh session.**
> It tells you where we are, what to read, and what to do next.

---

## Current State

| Field | Value |
|-------|-------|
| **Phase** | 4 — Compression Readiness (Checkpoint A complete) |
| **Status** | Phase 3 complete/remediated; Phase 4 Checkpoint A complete; manual verification in progress |
| **Last Updated** | 2026-02-14 |
| **Blocking Issues** | MCP smoke test currently hitting Anthropic orphan `tool_result` validation error |
| **Next Step** | Triage/fix Phase 3 tool lifecycle validation issue, then continue Phase 4 Checkpoint B. |

---

## Active Handoff (2026-02-13)

### Phase 4 Compression Readiness — Checkpoint A (NEW)

**Artifacts:**
- `dev/active/phase-4-compression-readiness/context.md`
- `dev/active/phase-4-compression-readiness/plan.md`
- `dev/active/phase-4-compression-readiness/tasks.md`

**Implemented (Checkpoint A):**
- Added `engine::compression` foundations:
  - `CompressionSettings` + backend/model default routing policy.
  - Provider contract with fail-open helper semantics.
  - Async queue contract (`CompressionQueue`, job lifecycle types).
- Wired compression settings into engine state + Tauri IPC:
  - `engine_get_compression_settings`
  - `engine_update_compression_settings`
- Added frontend settings/store support for sidekick compression config:
  - Backend select, model override, timeout, max tokens.
- Preserved fail-open proxy behavior and existing planner/rewrite boundaries (no autonomous sidekick execution yet).

**Validation (Checkpoint A):**
- `cargo fmt --check` ✅
- `cargo clippy -- -D warnings` ✅
- `cargo test` ✅ **504 total** (465 lib + 6 bin + 33 integration)
- `npx vitest run` ✅ **50/50**
- `npm run check` ✅ 0 errors, 0 warnings

**Next checkpoint:**
- Implement real provider adapters + queue worker execution (Checkpoint B), then integrate autonomous sidekick compression path.

**Current verification issue (2026-02-14):**
- MCP smoke test failed with Anthropic validation error:
  - `unexpected tool_use_id found in tool_result blocks ... must have a corresponding tool_use block in the previous message`
- Treat as Phase 3 lifecycle triage before deeper Phase 4 testing.

### Phase 3 Staff Review + Remediation Plan (NEW)

**Review artifacts:**
- `dev/active/metacog-dynamic-shifting/staff-review-2026-02-13.md` — full staff-level findings, severity-ranked
- `dev/active/metacog-dynamic-shifting/plan.md` — remediation waves and exit criteria
- `dev/active/metacog-dynamic-shifting/tasks.md` — implementation checklist
- `dev/active/metacog-dynamic-shifting/context.md` — continuity context
- `dev/active/metacog-dynamic-shifting/restart-prompt.md` — post-clear resume prompt

**Priority findings to fix first:**
1. Budget ceiling setting does not affect heuristic thresholds at runtime
2. Re-invoke loop ordering can strip required context-tool lifecycle state
3. Archive/compress/update semantics are not durably represented as between-turn engine state
4. Intercepted responses currently capture original upstream body instead of modified body

**Immediate plan:**
- Wave 1 complete (2026-02-13):
  - Fixed planner runtime budget ceiling plumbing into heuristics.
  - Fixed re-invoke lifecycle ordering to preserve context tool conversation state.
  - Fixed intercepted response capture body source to use effective returned body.
  - Added test coverage for context-only re-invoke, mixed calls, depth-limit fail-open, timeout fail-open, and budget ceiling override behavior.
- Wave 2 complete (2026-02-13):
  - Persisted archive/compress/update/expand semantics into engine-side durable mutation application.
  - Reordered capture/rewrite/ingest flow so capture occurs from effective rewritten payload semantics.
  - Wired planner signals from real proxy traffic (current files, previous files/task-boundary, file mutations).
  - Added round-trip persistence tests across multiple turns and capture-order regression coverage.
- Wave 3 complete (2026-02-13):
  - MCP `tools/list` schema is now generated from shared runtime tool definitions (no duplicated MCP-only schema surface).
  - Frontend threshold math aligned to planner policy (50/80/100 of configured ceiling).
  - `budgetCeiling` is now passed to `TokenBudgetBar` usage site.
  - Tool lifecycle integration tests now enforce rewrite/tool-array expectations (no weak optional assertions).
  - Existing Svelte warnings addressed in touched components (`ContextBlock`, `SettingsPanel`).
  - Validation: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `npx vitest run` (47/47), `npm run check` (0 warnings).
- Phase 4 kickoff completed: Checkpoint A foundations implemented and validated.

---

### Phase 3 Checkpoint G (COMPLETE)

**What was built:**

**G2/G6 — Engine HTTP API** (`src-tauri/src/proxy/context_api.rs`):
- `handle_aperture_route()` — Routes `/_aperture/*` requests to internal handlers
- 6 endpoints: `/context/{preview,read,search,plan,status}` + `/health`
- Each handler extracts engine blocks + budget, calls `dispatch_tool()`, returns JSON
- Health endpoint returns service status + version
- Error handling: 503 if engine unavailable, 400 for invalid JSON, 404 for unknown routes
- 8 tests covering all endpoint types, error cases, and routing

**Router Integration** (`src-tauri/src/proxy/handler.rs`, `mod.rs`):
- `/_aperture/` path check added before upstream routing in `proxy_handler()`
- `pub mod context_api` registered in `proxy/mod.rs`
- Internal routes never forwarded upstream

**G3 — MCP Server Binary** (`src-tauri/src/bin/aperture_mcp.rs`):
- Standalone `aperture-mcp` binary (stdio JSON-RPC 2.0, newline-delimited)
- MCP protocol: `initialize`, `notifications/initialized`, `tools/list`, `tools/call`, `ping`
- 5 context tools with MCP-format `inputSchema` definitions
- HTTP calls to proxy's `/_aperture/context/*` endpoints via `reqwest::blocking`
- Reads `APERTURE_PORT` env var (default 5400)
- Tool errors returned as MCP `isError: true`, not HTTP errors (fail-open)
- `[[bin]]` entry in Cargo.toml, `reqwest` blocking feature added
- 5 tests for tool path mapping, tool definitions, and schema validation

**G5 — ClaudeMcpRuntime Verification:**
- Existing `cleanup_history()` → `strip_anthropic_context_tools()` already handles MCP tool_use/tool_result stripping correctly
- No code changes needed — the Anthropic-format cleanup covers MCP messages natively

**Validation:**
- `cargo clippy -- -D warnings` ✅ clean
- `cargo fmt --check` ✅ clean
- `cargo test` ✅ **473 total** (440 lib + 5 bin + 2 session + 17 proxy + 9 tool lifecycle)
- `vitest` ✅ **44 frontend tests**
- `svelte-check` ✅ 0 errors, 2 warnings
- `cargo build --bin aperture-mcp` ✅ binary builds

**Test counts by checkpoint:**
| Checkpoint | New Rust Tests | New Frontend Tests | Cumulative Rust | Cumulative Frontend |
|-----------|---------------|-------------------|-----------------|---------------------|
| A (Planner + Tools) | 69 | 0 | 260 | 37 |
| B (Adapters + Cleanup) | 59 | 0 | 338 | 37 |
| C (Heuristics + File Tracking) | 53 | 0 | 371 | 37 |
| D (Rewriter + UI) | 50 | 0 | 421 | 37 |
| E (Tool Lifecycle) | 30 | 0 | 451 | 37 |
| F (Integration + Tests) | 9 | 7 | 460 | 44 |
| **G (MCP Server)** | **13** | **0** | **473** | **44** |

---

## MCP Configuration for Claude Code

To use the MCP server with Claude Code, add to project `.mcp.json`:

```json
{
  "mcpServers": {
    "aperture": {
      "command": "aperture-mcp",
      "env": {
        "APERTURE_PORT": "5400"
      }
    }
  }
}
```

Or auto-configure via `aperture claude`.

**Data flow:**
```
Claude Code ──(MCP stdio)──→ aperture-mcp binary
  aperture-mcp ──(HTTP)──→ http://localhost:5400/_aperture/context/preview
    proxy handler ──→ dispatch_tool("aperture_context_preview", ...) ──→ engine
  aperture-mcp ←── JSON result
Claude Code ←── MCP tool_result
```

---

## Key Reads for Next Session

1. `dev/active/phase-4-compression-readiness/context.md`
2. `dev/active/phase-4-compression-readiness/plan.md`
3. `dev/active/phase-4-compression-readiness/tasks.md`
4. `dev/active/metacog-dynamic-shifting/design.md`
5. `.context/phases/README.md`
6. `.context/phases/phase-4.md` (note: physical file naming is still shifted)

---

## Previous Handoffs

### Phase 3 Checkpoint F (COMPLETE)
- Settings UI, integration tests (9 Rust + 7 frontend), manual test playbook (8 scenarios)

### Phase 3 Checkpoint E (COMPLETE)
- Stream detection, tool injection gating, context tool interception + re-invoke loop, ~30 tests

### Phase 3 Checkpoint D (COMPLETE)
- Payload rewriter, mutation applicator, budget ceiling UI, ~50 tests

### Phase 3 Checkpoint C (COMPLETE)
- Autonomous heuristics, relevance scoring, file mutation tracking, 53 tests

### Phase 3 Checkpoint B (COMPLETE)
- Client adapters (Claude/Codex/Passive), ephemeral cleanup, 59 tests

### Phase 3 Checkpoint A (COMPLETE)
- Context Planner foundation, context tools, 69 tests

### Earlier Phases (COMPLETE)
- Phase Reorder + Metacognition Design (Session 14)
- Phase 2 Post-Stabilization (Sessions 11-13)
- Phase 1 + 1.5 (Sessions 1-10)

---

## Session Workflow

1. Read this file first
2. Read current phase/checkpoint docs
3. Continue from checkpoint
4. Update RESUME.md before compaction (~70% context)
