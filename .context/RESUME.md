# Aperture Resume Context

> **Read this file first when starting a fresh session.**
> It tells you where we are, what to read, and what to do next.

---

## Current State

| Field | Value |
|-------|-------|
| **Phase** | 4 — Token Economics Parity |
| **Status** | **Manual test Round 10 PASSED. 2 successful cleans. All 3 fixes implemented. Log analysis pending.** |
| **Last Updated** | 2026-02-20 |
| **Tests** | 696 passing (643 Rust + 53 frontend), clippy clean |
| **Next Step** | Analyze Round 10 JSONL logs — verify diagnostic tracing, confirm H1 vs H2, assess remaining bugs |

---

## Manual Test Round 10 Results (2026-02-20)

**Best run yet. 2 successful context cleans with plan layering.**

### MCP Tool Coverage — All Confirmed Working

| Tool | Status |
|------|--------|
| `aperture_context_preview` | ✓ returns zone-grouped block list with archival suggestions |
| `aperture_context_status` | ✓ returns full manifest with token counts per block |
| `aperture_context_search` | ✓ returns relevance-ranked matches with snippets + file paths |
| `aperture_context_read` | ✓ returns full block content; output guardrail truncates at ~5.8k chars |
| `aperture_context_plan` | ✓ stage → commit flow works; all ops confirmed below |

### Plan Operations — All Confirmed Working

| Op | Result |
|----|--------|
| `archive` | Strips blocks from payload, adds to `persistent_archived_ids` |
| `compress` | Replaces block content with model-authored summary (-3.7k for 3.8k block) |
| `expand` | Restores full content from compressed block |
| `recall` | Brings archived block back into active context |
| `pin` | Marks block to prevent auto-archival |
| `shift_to` | Moves block to target zone (tested: Middle → Primacy) |

### Confirmed Behaviors

- **One-turn lag**: Plan committed in turn N fires on turn N+1's outgoing request. Expected.
- **Persistent archival stacking**: Multiple archive rounds accumulate correctly. Round 1 (8 blocks) + Round 2 (8 blocks) + Round 3 (5 blocks) = 21 blocks persistently stripped each turn.
- **Plans layered with user turns between them work correctly** — second and third plans fire as expected when each commit has at least one user turn before the next commit.

### Remaining Bugs (Low Severity)

1. **Net: +0 in breadcrumb** — delta always shows 0 for persistent re-archival. Engine marks blocks archived before breadcrumb delta is calculated. Budget % IS correct. Only the delta display is wrong.
2. **Budget % mismatch vs `/context`** — breadcrumb computes from engine's message payload only (~22% for 44k messages). `/context` includes overhead (system prompt 8.6k + system tools 17.6k + MCP tools 3.6k + memory 5.6k = ~35k). At 200k limit, this ~35k overhead is a constant ~17.5% gap.
3. **Claude Code crash on file edit through proxy** — When CC edits files while running through Aperture, it crashes after the edit lands. Edits persist but the session dies — user must `cd` back and `/resume`. Model doesn't know the edit succeeded (interrupted before response logged). Seen in R9 and R10. Likely related to R9-2 (file-watcher crash). May be CC bug triggered by proxy latency/rewriting, not Aperture logic.

### Key Observation: Tool Overhead Cost

Each full plan cycle (preview + stage + commit) adds ~2-3k tokens in tool use/result blocks. Multiple operations in one session can add 12-16k tokens — partially offsetting archival savings.

**Practical guideline**: Target blocks >3k tokens for manual archival. Archiving a 500-token block costs more in tool overhead than it saves.

---

## Implemented Fixes (2026-02-20)

All 3 fixes implemented, tested (696 total), clippy clean.

| # | Fix | Severity | Status |
|---|-----|----------|--------|
| 1 | **Option B**: `add_persistent_archives_for_session()` + call at commit time | P0 | **IMPLEMENTED** |
| 2 | **Diagnostic tracing**: 4 `warn!()` calls (rewriter cold-start, rewriter consume, context_api, planner) | P0 | **IMPLEMENTED** |
| 3 | **MCP retry**: `call_proxy()` retries once on connection failure (500ms delay) | P2 | **IMPLEMENTED** |

### Code Changes

- `engine/planner/mod.rs:237-262` — New `add_persistent_archives_for_session()` method
- `metacog/tools/plan.rs:248` — Call `add_persistent_archives_for_session()` after commit
- `proxy/rewriter.rs:69-74` — R9-DIAG: cold-start with pending plan (H2 indicator)
- `proxy/rewriter.rs:116-121` — R9-DIAG: pending plan consumed (session + mutation count)
- `proxy/context_api.rs:159-171` — R9-DIAG: plan stage/commit session resolution
- `engine/planner/mod.rs:520-525` — R9-DIAG: plan applied in planner (session + persistent count)
- `mcp/server.rs:64-89` — Retry loop (2 attempts, 500ms delay, stderr logging)

### New Tests (3)

- `test_add_persistent_archives_at_commit_time` — verifies persistent IDs survive without plan_for_session consumption
- `test_add_persistent_archives_recall_removes_from_persistent_set` — recall correctly removes from persistent set
- `test_add_persistent_archives_idempotent` — duplicate inserts produce exactly one mutation

---

## Next Session: Log Analysis

1. **Read this file** (already reading)
2. **Find Round 10 JSONL log** in `~/.claude/projects/-home-caden-projects-Aperture/`
3. **Analyze diagnostic traces**: grep for `R9-DIAG` in proxy stderr/logs
4. **Confirm H1 vs H2** — compare session IDs across trace points
5. **Assess breadcrumb delta bug** — determine fix approach
6. **Assess budget % gap** — determine if overhead should be included in engine budget

### If H1 confirmed (session mismatch)
Consider session resolution hardening — single canonical session path.

### If H2 confirmed (streaming race)
Consider atomic ingest or mutex around block store during ingest.

### After root cause confirmed
Downgrade `R9-DIAG` tracing from `warn!()` to `debug!()`.

---

## Key Architecture (Post-Refactor)

- **Parser** (`proxy/parser/*`) — wire parsing → canonical `Block` records
- **Rewriter** (`proxy/rewriter/*`) — JSON mutation, cleanup, trailing injection
- **Planner** (`engine/planner/*`) — mutation planning, staged plans, heuristics
- **Engine** (`engine/`) — session/block state, ingest, persistence, policy
- **Handler** (`proxy/handler/*`) — upstream routing, transport filtering
- **Interceptor** (`proxy/interceptor/*`) — context-tool interception, reinvoke
- **Capture** (`proxy/capture/*`) — capture store, SSE reconstruction
- **MCP** (`mcp/*`) — JSON-RPC transport, tool routing, session affinity
