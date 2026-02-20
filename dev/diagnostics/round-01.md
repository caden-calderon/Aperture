# Deep Dive Diagnostics Round 1 (2026-02-19)

## Scope
- Mode: diagnostics and analysis only.
- No code fixes in this round.
- Goal: tighten problem statements, isolate root-cause candidates, and define proof requirements before any implementation work.

## Test Run Anchors
- Claude session log: `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl`
- Aperture DB: `~/.aperture/aperture.db`
- User repro baseline:
  - starts with `claude!`
  - manual context fill
  - context plan stage/commit
  - `/context` checks showing 64% -> 67% -> 69%
  - UI symptoms: temporary block disappear/reappear, archive notification spam, token mismatches

## Severity-Ordered Findings (Current Confidence)

### P0-1: Commit can report large archival savings while payload removal is effectively zero
Confidence: High

Evidence:
- Commit staged 7 archive IDs and reported projected `-61k`.
- In session `151c97a1-cf86-497b-9b58-dac5433d1429`, archived IDs only partially covered turns:
  - turn 5: 4/6 blocks archived
  - turn 7: 3/4 blocks archived
- Turn removal requires all blocks in the turn to be archived:
  - `src-tauri/src/engine/planner/applicator.rs:190`
- Planner projection uses raw archive mutation count/tokens, not actual rewrite-feasible removals:
  - `src-tauri/src/engine/planner/validation.rs:235`

Impact:
- Breadcrumb/assistant can claim major savings while Claude `/context` does not drop.
- Strongly explains "commit not working" user perception.

---

### P0-2: Context tool cleanup likely misses Claude MCP namespaced tool names
Confidence: High

Evidence:
- Tool names in real run include:
  - `mcp__aperture__aperture_context_preview`
  - `mcp__aperture__aperture_context_plan`
- Cleanup name check is prefix-only on canonical names:
  - `aperture_context_`
  - `src-tauri/src/metacog/runtime.rs:57`
- Anthropic cleanup uses that matcher:
  - `src-tauri/src/engine/planner/cleanup.rs:82`

Impact:
- Context tool calls/results can persist in request history instead of being stripped.
- Increases message payload and can trigger repeated planning churn.

---

### P0-3: Active session churn can drive UI disappear/reappear and false "archived" toasts
Confidence: High

Evidence:
- Session create/switch sets active session immediately:
  - `src-tauri/src/engine/session.rs:115`
  - `src-tauri/src/engine/mod.rs:863`
- UI refresh uses active session every context update:
  - `src/routes/+page.svelte:69`
- UI block list is always active-session blocks:
  - `src-tauri/src/lib.rs:116`
- Toast logic marks any old->new missing IDs as archived, with no session-change guard:
  - `src/lib/stores/context.svelte.ts:861`
- DB around repro window shows many short-lived side sessions (including tiny Haiku topic-classification sessions and one-block Opus memory sessions).

Impact:
- Blocks can appear to vanish and reappear when active session flips.
- Archive notification spam can be false-positive UI interpretation.

---

### P1-1: Token bar/Aperture metrics mismatch is currently structural
Confidence: High

Evidence:
- Backend budget includes overhead tokens:
  - `src-tauri/src/engine/mod.rs:184`
- UI requests only `limit_tokens` from backend budget call:
  - `src/routes/+page.svelte:72`
- UI budget used is calculated from block sums only:
  - `src/lib/stores/context.svelte.ts:248`
  - `src/lib/mock-data.ts:612`

Impact:
- Token bar, MCP readouts, and Claude `/context` can disagree even without a runtime bug.

---

### P1-2: Context API default session targeting is vulnerable under active-session instability
Confidence: Medium-High

Evidence:
- Tool session resolve falls back to active session unless `_aperture_session_id` provided:
  - `src-tauri/src/proxy/context_api.rs:206`
- MCP plan affinity exists but does not globally guarantee every context tool call is pinned.

Impact:
- Preview/read/search/status can target the wrong session during churn windows.

## Ruled Out In This Round
- "Engine captures pre-rewrite raw payload and re-inserts archived content" as primary cause.
  - Handler captures from effective body after rewrite path selection:
    - `src-tauri/src/proxy/handler.rs:439`
    - `src-tauri/src/proxy/handler.rs:470`

## Open Questions (Need Further Proof)
- Exact causality chain for side-session creation volume:
  - fallback identity instability vs expected classifier/memory traffic vs both.
- Whether namespaced tool cleanup miss alone explains full archive-notification behavior.
- Whether persistent archival + turn-level removal semantics should be treated as design mismatch vs defect.
- Whether stage/commit affinity is always pinned across all call sequences in mixed tool workflows.

## Pre-Planning (No Concrete Fix Commitments Yet)

### Investigation Track A: Session Identity and Active Session Stability
- Build event timeline across:
  - parsed thread identity
  - resolved session ID
  - active session ID transitions
  - emitted `context_updated` events
- Goal: prove exact mechanism for disappear/reappear and quantify how often active session shifts per user turn.

### Investigation Track B: Cleanup Name Normalization
- Reproduce cleanup behavior using captured tool names from real logs.
- Add proof tests first that fail on `mcp__aperture__aperture_context_*` names.
- Goal: determine if cleanup mismatch is a definitive bug and isolate blast radius.

### Investigation Track C: Projection vs Applied Rewrite Delta
- Build a deterministic replay case:
  - stage archive set with partial-turn coverage
  - commit
  - compare projected delta vs actual payload delta
- Goal: decide whether this is an algorithm bug, messaging bug, or both.

### Investigation Track D: Token Metrics Reconciliation Model
- Define and document three distinct numbers:
  - Aperture blocks-only tokens
  - Aperture effective tokens (with overhead)
  - Claude `/context` tokens
- Goal: classify "expected divergence" vs "unexpected drift".

### Investigation Track E: Web/Official Docs Deep Pass (Next Round)
- Claude Code docs:
  - MCP tool invocation and session behavior
  - `/context` accounting categories
  - caching behavior notes
- Anthropic docs:
  - prompt cache prefix invalidation boundaries
  - tool/history shape constraints
- Codex/OpenAI docs:
  - responses/chat tool history semantics
  - cache/prefix or equivalent mechanics
- Goal: verify whether Aperture assumptions about history rewriting and token accounting align with upstream client/platform behavior.

## TDD and Proof Gates Before Any Fixing
- Gate 1: failing regression test exists for claimed bug.
- Gate 2: bug mechanism is directly observed in runtime evidence (logs/DB/request traces).
- Gate 3: passing test proves corrected behavior without collateral regressions.
- Gate 4: manual script replay confirms real-world symptom reduction.

## Next-Round Evidence Collection Checklist
- Fresh runtime logs from the exact failing run (not stale `/tmp` files).
- Session-diff report across active session transitions during tool-heavy turns.
- Extracted list of tool names from live payloads for cleanup normalization validation.
- Replay harness for stage/commit archival with partial-turn archives.
- Side-by-side metric capture:
  - Aperture UI
  - Aperture context tools
  - Claude `/context`

## Operating Rule
- Continue in diagnostics mode until a candidate reaches "without a doubt" confidence by evidence + tests.
- No speculative fix merges.
