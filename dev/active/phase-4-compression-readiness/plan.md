# Phase 4 Token Economics Plan (2026-02-15, revised 2026-02-14)

## Staff Remediation Plan (2026-02-18)

Status: Implemented.

### Session-Scoped Planner State (Hard Requirement)
Options considered:
1. Thread-local planner mutable state.
   - Rejected: does not isolate concurrent sessions sharing worker threads; brittle across async boundaries.
2. Engine-owned per-session planner state map (selected).
   - Chosen: key planner mutable state by resolved session identity (`provider:model:source:thread`) and pass session IDs through rewrite/tool paths.
3. Persist planner mutable state in storage/session records only.
   - Rejected for now: heavier I/O path for high-frequency planner operations; unnecessary for current correctness gate.

Implementation decision:
- Keep a shared planner correctness contract but move all mutable planner fields to per-session buckets.
- Preserve runtime-specific optimizations (MCP/proxy differences) without changing correctness semantics.
- Add explicit session-state cleanup on session reset/clear paths.

### Suggestion Policy Contract (Hard Requirement)
- Tier A (default): stale + middle-zone suggestions only.
- Tier B (opportunistic): recency suggestions only when:
  - task boundary is detected,
  - pressure is `Critical` or `Emergency`,
  - candidate is unpinned and low-relevance.
- Tier B is excluded from stale warning counts and stale warning language.

### Additional Correctness Remediations (Implemented)
- OpenAI trailing warning/breadcrumb parity across valid payload variants (chat string/array content; responses string/object/array input).
- Projected block-count robustness with dedup + saturating projection math (no underflow on duplicate archive/recall mutations).
- `context_preview` now uses real planner/session signals instead of synthetic empty signals.
- Restored quality gate: `cargo clippy -- -D warnings` passing.

## Refactor-First Pivot Plan (2026-02-19)

Status: Refactor tranches #1, #2, and #3 complete, plus post-tranche hygiene pass (context_api cleanup + docs navigation consolidation). Diagnostics rounds #2 and #3 completed with fresh repro evidence and replay tests (no production fixes). Next phase is targeted bug-fix deep dive from the cleaner baseline.

### Problem Statement
- User-reported behavior remains effectively unchanged after multiple targeted fix attempts.
- Large mixed-concern files and inconsistent code standards are increasing debugging cost and reducing patch reliability.
- Current loop of patch -> manual test -> partial/no improvement is not converging.

### Strategy
1. Pause new feature/bug work briefly.
2. Run staff-level cleanup/refactor pass on core paths first.
3. Re-attack functional bugs from a cleaner, better-factored baseline.

### Refactor Scope
- Engine/proxy/planner/parser/rewriter/metacog MCP path.
- Extract tests from high-churn runtime files into dedicated test modules/files where practical.
- Split oversized files by responsibility (parse, mutate, apply, event/dispatch, budgeting).
- Remove dead or duplicated logic branches.
- Standardize naming/error/logging patterns.

### Concrete Refactor Map (Locked)
- `parser` ownership: provider wire JSON parsing -> canonical `Block` projection.
- `rewriter` ownership: canonical mutation decisions -> JSON patch/sanitize/inject.
- `planner` ownership: mutation planning + staged/pending lifecycle + heuristic signal policy.
- `engine` ownership: session/block authority, ingest, persistence, internal mutation apply.

File-level tranche map:
- `src-tauri/src/proxy/parser.rs` -> `src-tauri/src/proxy/parser/mod.rs` + focused sibling modules.
- Move parser inline tests out of runtime file into `src-tauri/src/proxy/parser/tests.rs`.
- Keep existing public parser API stable (`parse_request*`, `parse_response`, provider path helpers).

### Tranche #1 Outcome (2026-02-19)
- Completed behavior-preserving parser decomposition:
  - `src-tauri/src/proxy/parser/mod.rs`
  - `src-tauri/src/proxy/parser/anthropic.rs`
  - `src-tauri/src/proxy/parser/openai.rs`
  - `src-tauri/src/proxy/parser/identity.rs`
  - `src-tauri/src/proxy/parser/overhead.rs`
  - `src-tauri/src/proxy/parser/tests.rs`
- Validation passed:
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- Behavior changes intentionally avoided; no new bug-targeted logic added in this tranche.

### Tranche #2 Outcome (2026-02-19)
- Completed behavior-preserving decomposition of remaining hotspots:
  - `src-tauri/src/proxy/rewriter.rs` -> `src-tauri/src/proxy/rewriter/{signals,sanitize,trailing,payload,tests}.rs`
  - `src-tauri/src/engine/mod.rs` ingest/regression extraction -> `src-tauri/src/engine/ingest.rs`
  - `src-tauri/src/engine/mod.rs` session sync/persistence extraction -> `src-tauri/src/engine/session_sync.rs`
  - `src-tauri/src/engine/mod.rs` inline tests -> `src-tauri/src/engine/tests.rs`
  - `src-tauri/src/engine/planner/mod.rs` validation/projection extraction -> `src-tauri/src/engine/planner/validation.rs`
  - `src-tauri/src/engine/planner/mod.rs` inline tests -> `src-tauri/src/engine/planner/tests.rs`
  - `src-tauri/src/metacog/tools.rs` plan-control extraction -> `src-tauri/src/metacog/tools/plan.rs`
  - `src-tauri/src/metacog/tools.rs` inline tests -> `src-tauri/src/metacog/tools/tests.rs`
- Validation passed:
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- Behavior changes intentionally minimized; no new bug-targeted feature work added in this tranche.

### Tranche #3 Outcome (2026-02-19)
- Completed behavior-preserving decomposition of remaining backend orchestration hotspots:
  - `src-tauri/src/bin/aperture_mcp.rs` -> thin entrypoint, runtime moved to `src-tauri/src/mcp/server.rs`, tests moved to `src-tauri/src/mcp/tests.rs`.
  - `src-tauri/src/proxy/handler.rs` -> orchestration retained; routing/header/exchange ownership moved to `src-tauri/src/proxy/handler/{routing,headers,exchange}.rs`; tests moved to `src-tauri/src/proxy/handler/tests.rs`.
  - `src-tauri/src/proxy/interceptor.rs` -> reinvoke/interception orchestration retained; response-shape ownership moved to `src-tauri/src/proxy/interceptor/response.rs`; tests moved to `src-tauri/src/proxy/interceptor/tests.rs`.
  - `src-tauri/src/proxy/capture.rs` -> capture lifecycle retained; SSE reconstruction moved to `src-tauri/src/proxy/capture/sse.rs`; tests moved to `src-tauri/src/proxy/capture/tests.rs`.
- Architecture/doc outputs:
  - ownership map refreshed for handler/interceptor/capture/MCP boundaries,
  - phase docs and task tracking updated for post-tranche bug-dive handoff.
- Validation passed:
  - `cargo test --manifest-path src-tauri/Cargo.toml`,
  - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`,
  - frontend untouched (no frontend checks required).

### Post-Tranche Hygiene Outcome (2026-02-19)
- Additional backend boundary cleanup:
  - `src-tauri/src/proxy/context_api.rs` refactored for clearer route matching, argument parsing, and response helpers.
  - inline tests moved to `src-tauri/src/proxy/context_api/tests.rs`.
- Repository/documentation organization cleanup:
  - added canonical docs index (`docs/DOCS_INDEX.md`) and repo ownership map (`docs/REPO_STRUCTURE.md`),
  - added folder-level indexes (`dev/active/README.md`, `.context/README.md`),
  - corrected stale references in `README.md`, `docs/ARCHITECTURE.md`, and `docs/INTEGRATION.md`.
- Validation passed:
  - `cargo test --manifest-path src-tauri/Cargo.toml`,
  - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`.

### Hackathon Docs/Repo Polish Outcome (2026-02-19)
- Documentation lifecycle and navigation tightened:
  - added `docs/DOC_LIFECYCLE.md`,
  - added `docs/HACKATHON_SUBMISSION.md`,
  - added archive indexes at `docs/archive/README.md` and `.context/archive/README.md`.
- Context root cleanup:
  - moved stale whimsical session notes to `.context/archive/`.
- Handoff quality:
  - added `.context/final-hackathon-polish-prompt.md` for clean post-clear continuation.

### Final Hackathon Polish Follow-Through (2026-02-19)
- Closed remaining plan-validation gap:
  - `src-tauri/src/metacog/tools.rs` now validates `aperture_context_plan` against normalized arguments so stringified arrays/maps and `#`-prefixed IDs are accepted consistently before dispatch.
  - Added dispatch-level regression tests in `src-tauri/src/metacog/tools/tests.rs`.
- Enforced one clear fresh-context path:
  - archived completed `.context/tranche-3-kickoff-prompt.md` under `.context/archive/`,
  - refreshed `.context/README.md` and `.context/archive/README.md`.
- Validation rerun:
  - `cargo test --manifest-path src-tauri/Cargo.toml`,
  - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`.

### Severity-Ordered Findings (Post Tranche #3)
- `RESOLVED (HIGH)`: `src-tauri/src/proxy/rewriter.rs` concern split completed.
- `RESOLVED (HIGH)`: `src-tauri/src/engine/mod.rs` ingest/session-sync split + test extraction completed.
- `RESOLVED (HIGH)`: `src-tauri/src/engine/planner/mod.rs` validation/projection split + test extraction completed.
- `RESOLVED (HIGH)`: proxy orchestration hotspots (`handler`, `interceptor`, `capture`) split by ownership with externalized tests.
- `RESOLVED (MEDIUM)`: `src-tauri/src/metacog/tools.rs` plan-control split + test extraction completed.
- `RESOLVED (MEDIUM)`: `src-tauri/src/proxy/context_api.rs` helper boundary tightening + test extraction completed.
- `RESOLVED (MEDIUM)`: MCP runtime ownership moved from bin hotspot into dedicated library module with tests.
- `RESOLVED`: parser hotspot split + test extraction completed in tranche #1.

### Guardrails
- Prefer behavior-preserving refactors.
- Any non-trivial behavior changes require explicit rationale and tests.
- Keep the system shippable at each step (no big-bang rewrite).
- Validate continuously with Rust and frontend checks after each tranche.

### Exit Criteria for Refactor Tranche
- Reduced average file size and lower concern-mixing in hot modules.
- Equivalent or improved test coverage with tests easier to target.
- Clear architecture map for ownership boundaries.
- Team confidence that bug-fix iteration can resume with less churn.

## Pivot Objective (Hard Gate)
Prove Aperture is net token-saving versus baseline Claude Code usage on representative tasks.

Success criterion (must pass before resuming feature expansion):
- `tokens_with_aperture <= tokens_without_aperture` (target: slightly below baseline)
- measured per task and aggregated across the benchmark suite.

No-go criterion:
- If Aperture remains net-positive token cost after mitigation work, pause expansion and re-scope architecture.

## Decision: Freeze Previous Expansion Track
Checkpoint A remains complete. Previous Checkpoint B/C/D expansion (provider adapters + autonomous compression rollout) is paused.

Rationale:
- Adding more autonomous behavior before economic proof risks amplifying token burn.
- Product viability depends on cost outcomes first, not feature breadth.

---

## Root Cause Analysis (Corrected 2026-02-14)

### Misdiagnosis Corrected
The initial plan focused on "large tool response payloads" as the root cause. Code analysis shows tool responses are already capped at 2-8KB (normal/compact modes). The actual cost drivers are:

### Primary Cost Driver: Cache Prefix Invalidation (CONFIRMED)
Aperture modifies request bodies in ways that may invalidate Anthropic's prompt cache prefix:
- **Manifest injection** into system message: changes on every request (block count, budget %) → cache miss from byte 0
- **Tool schema injection**: adds/modifies tool definitions → cache miss on tool block
- **Block archival/removal**: changes conversation structure → cache miss from removal point

Cache invalidation math (per request, 100k token prefix):
- Cache hit (baseline): 100k × 0.1 = 10k effective tokens
- Cache miss (Aperture-modified): 100k × 1.25 = 125k effective tokens
- Delta: 115k effective tokens per request of pure overhead
- Over 100 requests: 11.5M effective tokens of unnecessary cache_create

Evidence confirming:
- Session `401b10df`: 46 requests, 5.34M cache_creation, **0 cache_read** — cache NEVER hit
- Session `88e1b95d`: high cache_read (~188-196k/req) suggesting stable prefix
- Anthropic docs confirm: system message change invalidates system AND messages cache
- "Don't Break the Cache" paper confirms: dynamic content in system prompts invalidates cached prefixes
- Code confirmed: `inject_tools()` is NO-OP for ClaudeMcpRuntime (tools NOT injected into API request)
- Code confirmed: manifest injection has NO streaming gate (busts cache on ALL requests)
- Code confirmed: zero `cache_control` awareness anywhere in Aperture codebase
- Full analysis: `cache-invalidation-analysis.md`

### Secondary Cost Driver: Per-Request Schema Overhead
5 tool definitions injected into every qualifying request:
- ~200-250 tokens per tool definition × 5 tools = ~1000-1250 tokens/request
- Over 375 requests: 375k-468k tokens of pure overhead
- Even at cache-read rates (if prefix stable), this is measurable

### Tertiary Cost Driver: Re-invocation Round-Trips
Each context-tool-only re-invocation costs full conversation prefix:
- 100k-200k tokens of cache_read + new content creation
- Max 3 per request × 60s timeout
- Rare in observed sessions (0 aperture tool calls in highest-burn sessions)

### What Was NOT the Problem
- Tool response payloads (already capped at 2-8KB)
- Aperture archive persistence blowing up (confirmed: not the main driver)
- Runaway context tool loops (0 context tool calls in highest-burn sessions)

---

## Corrected Priority Stack

```
P0: DONE — Cache invalidation confirmed as primary cost driver (see cache-invalidation-analysis.md)
P1: Remove manifest injection from system message (THE fix — eliminates $1.12/request overhead)
P2: Economics ledger (measure to prove fix works)
P3: Cache-stable archival strategy (stable decisions, append-only where possible)
P4: Schema overhead reduction for non-MCP paths (Codex/OpenAI)
P5: ROI controller + auto-degrade (safety net)
P6: Benchmark suite (prove parity)
P7: Delta protocol (third-order optimization, do last)
```

---

## Architecture Options Considered

### Option A: Guardrails-only hardening (rejected as primary)
- Keep current APIs and rely on caps/rate limits/circuit breakers.
- Pros: low effort, immediate damage control.
- Cons: does not change core cost model; only suppresses worst cases.

### Option B: Delta protocol for context tools (deprioritized — was overweighted)
- Replace full/snapshot-style responses with revisioned deltas.
- Pros: reduces repeated payload cost when tools are called multiple times.
- Cons: addresses third-order cost; tool responses already capped; doesn't fix cache invalidation.

### Option C: Cache-stable request construction (NEW — highest priority if hypothesis confirmed)
- Ensure Aperture's modifications preserve Anthropic prompt cache prefix stability.
- Move dynamic content (manifest, breadcrumbs) AFTER the stable prefix.
- Append-only modification strategy: never modify existing content, only append/remove from end.
- Pros: directly addresses highest-cost overhead source.
- Cons: requires understanding Anthropic's exact cache key behavior.

### Option D: Proactive injection replacing reactive tools
- Move preview/status to proactive system message injection (no tool calls needed).
- Keep only read/search/plan as actual tools.
- Pros: eliminates most common re-invocation trigger.
- Cons: adds to system message (potential cache impact — must be in stable position).

### Option E: Passive-default with explicit opt-in
- Run passive unless operator enables active mode for specific sessions.
- Pros: safest cost posture.
- Cons: weakens product utility if always manual; nuclear option.

## Selected Strategy
1. **Validate cache hypothesis** (P0) before committing to architecture.
2. **Build economics ledger** (P1) to measure actual costs.
3. **Cache-stable construction** (P2) if hypothesis confirmed.
4. **Schema reduction + lazy injection** (P3) for fixed-cost reduction.
5. **Proactive injection** (P4) to eliminate re-invocations.
6. Delta protocol, ROI controller, benchmark as follow-through.

---

## Detailed Cost Model

### Per-Request Overhead (Fixed)
| Item | Tokens | When | Fixable? |
|------|--------|------|----------|
| Tool schema injection | ~1000-1250 | Every request with >3 non-system blocks | Yes: consolidate/lazy inject |
| Manifest injection | ~30-50 | Every request with blocks | Yes: move to stable position |
| Breadcrumb injection | ~20-80 | Per mutation turn | Yes: move to stable position |
| **Cache invalidation** | **~115k effective** | **If prefix changes (every modified request)** | **Yes: cache-stable construction** |

### Per-Event Overhead (Variable)
| Item | Tokens | When | Fixable? |
|------|--------|------|----------|
| Context tool response | 800-5000 | Per tool call | Partially: delta protocol |
| Re-invocation | Full prefix (100-200k cache_read) | Per context-only response | Yes: proactive injection |

### Per-Request Savings
| Item | Tokens Saved | When |
|------|-------------|------|
| Block archival | archived_block_tokens | Every subsequent request |
| Block compression | (original - compressed) tokens | Every subsequent request |
| Context tool cleanup | stripped tool_use/tool_result tokens | Every subsequent request |
| Orphan cleanup | stripped invalid blocks | Per affected request |

### Break-Even Analysis
For Aperture to be net-positive (ignoring cache effects):
- Schema overhead: ~1200 tokens/request
- Need to archive: ≥1200 tokens of blocks to break even per request
- A single medium tool_result (500-5000 tokens) archived pays for itself

Including cache effects (if invalidation hypothesis confirmed):
- Cache invalidation: ~115k effective tokens/request
- Need to archive: ≥115k tokens to break even ← **impossible for typical sessions**
- This means cache stability is non-negotiable for viability

---

## Implementation Sequence

### Phase 4.1: Measurement + Cache Investigation
1. Validate cache invalidation hypothesis (research + controlled test)
2. Build economics ledger (instrument all cost sources)
3. Expose economics via `/_aperture/economics` endpoint

### Phase 4.2: Root Cause Fix
4. Cache-stable request construction (if hypothesis confirmed)
5. Schema consolidation (5 tools → 1 or 2-3, lazy injection)
6. Proactive injection (preview/status → system message, no tools needed)

### Phase 4.3: Optimization + Validation
7. Delta protocol for remaining tool responses
8. ROI controller with auto-degrade
9. Benchmark suite + acceptance report

### Phase 4.4: Resume Expansion (Only After Parity Gate)
10. Revisit paused Checkpoint B/C/D roadmap items

---

## Non-Negotiable Constraints
- Do not regress Phase 3 tool lifecycle correctness.
- Keep proxy forwarding fail-open for non-context traffic.
- Avoid blocking I/O in proxy request/response path.
- Any new token-saving claim must be validated by benchmark evidence.
- Cache prefix stability must be validated, not assumed.

---

## Failure Modes and Contingencies

| Failure Mode | Mitigation | Nuclear Option |
|---|---|---|
| LLM ignores context tools entirely | Lazy injection prevents schema waste | Remove tool injection entirely |
| LLM calls context tools obsessively | Circuit breaker + ROI controller | Auto-passive mode |
| Short sessions dominate | Lazy injection gate (no injection on short sessions) | Product thesis wrong for short sessions |
| Cache invalidation confirmed and unfixable | Append-only modification strategy | Passive-only mode |
| Parity met but value is zero | Phase 4 compression must show savings | Pivot to visualization-only product |
| Proactive injection distracts LLM | Conditional on session length/budget | Remove proactive injection |
