# Phase 4 Token Economics Tasks (2026-02-14)

## Staff Review Remediation (2026-02-18)
- [x] Session isolation: planner mutable state is fully session-scoped across rewrite/tool/ingest flow
- [x] Session identity propagation: parser/capture/engine/rewriter/context API use consistent thread-aware session keys
- [x] Two-tier suggestion policy:
  - [x] Tier A default = stale + middle-zone only
  - [x] Tier B opportunistic recency gated by task-boundary + Critical/Emergency + unpinned + low-relevance
  - [x] Tier B excluded from stale warning counts/language
- [x] Projected block-count robustness:
  - [x] dedup mutation targets before projection
  - [x] saturating projection arithmetic to prevent underflow
  - [x] regression tests for duplicate archive/recall behavior
- [x] OpenAI trailing warning/breadcrumb parity for valid payload variants (chat + responses shapes)
- [x] `context_preview` signal quality fix (real session signals, not synthetic empty signals)
- [x] Shared correctness contract preserved across runtimes, runtime-specific optimizations retained
- [x] Restore full quality gates including `cargo clippy -- -D warnings`
- [x] Add session-isolation integration tests and suggestion-tier policy tests

## Checkpoint A: Foundations (COMPLETE)
- [x] Define Rust config types for compression backend/model selection
- [x] Add provider-aware default routing policy for sidekick model selection
- [x] Add compression provider trait with fail-open helper semantics
- [x] Add async compression queue contract types/state machine
- [x] Add engine-owned compression settings getter/setter
- [x] Define Tauri IPC for reading/updating compression settings
- [x] Add UI placement and UX copy for compression sidekick settings
- [x] Add backend-failure fail-open tests and settings normalization tests

## P0: Cache Invalidation Investigation (CONFIRMED)
- [x] Research Anthropic prompt caching key behavior (docs + web)
- [x] Research Claude Code's cache_control breakpoint placement strategy
- [x] Determine: does modifying system message bust entire cache prefix? → YES (system + all messages)
- [x] Determine: does modifying tool definitions bust cache independently? → YES (tools + system + messages)
- [x] Determine: does removing conversation-middle blocks bust cache from that point? → YES (from removal point onward)
- [x] Confirm inject_tools() is NO-OP for ClaudeMcpRuntime (tools NOT the problem for Claude Code)
- [x] Confirm manifest injection has NO streaming gate (busts ALL requests)
- [x] Confirm zero cache_control awareness in Aperture codebase
- [x] Document findings → `cache-invalidation-analysis.md`
- [ ] Run controlled A/B test: same task, Aperture passive vs active, compare cache_create/cache_read

## Manual Test Fixes (2026-02-16)

### Pre-Test Fixes (COMPLETE)
- [x] Fix 1: Token-proportional zone assignment (zone.rs rewrite)
- [x] Fix 2: Tool block archival gating at Critical+ pressure (heuristics.rs)
- [x] Fix 3: Token-based archival targets replacing count caps (heuristics.rs)
- [x] Fix 4: ANSI escape code stripping (util.rs + engine/mod.rs)
- [x] Fix 5: Internal prompt filter for Claude Code noise blocks (engine/mod.rs)
- [x] All tests passing: 557 Rust + 52 frontend

### Post-Test Fixes (COMPLETE)
- [x] Fix A: Remove Thinking→Primacy in zone.rs (let thinking follow token-proportional rules)
  - Removed Role::Thinking from 4 code locations + 2 doc comments
  - 2 new zone tests (small + large context)
- [x] Fix B: Budget overhead tracking (extract tool array tokens from request JSON, add to budget)
  - [x] parser.rs: Added overhead_tokens to ParsedRequest, estimate_tool_overhead() helper, 4 tests
  - [x] capture.rs: Added overhead_tokens to CapturedExchange
  - [x] session.rs: Added overhead_tokens to Session + SessionInfo
  - [x] engine/mod.rs: Accept overhead in ingest(), include in budget_status(), 3 tests
  - [x] handler.rs + codex_bridge.rs: Wired overhead through call sites
- [x] Fix C: Orphan tool_use sanitizer (discovered from manual test log analysis)
  - Log analysis revealed 3x 400 errors from orphan tool_use in assistant messages
  - Added sanitize_anthropic_orphan_tool_uses() — reverse direction of existing tool_result sanitizer
  - Wired into both cold-start and main rewrite paths
  - 7 tests (removes orphan, keeps valid pairs, preserves pending, partial strip, mid-gap, cold-start)
- [x] Fix E: Plan stage session affinity pinning (mixed re-test bugfix, 2026-02-19)
  - `aperture_mcp.rs`: `with_plan_session_hint()` now includes `stage` op
  - Prevents `context_plan(stage)` from resolving against rotated `active_session_id`
  - `aperture-mcp` bin tests updated/passing
- [x] Fix F: Regressive semantic collapse guard (mixed re-test bugfix, 2026-02-19)
  - `engine/mod.rs`: added semantic ingest guard for severe transient collapses when IDs churn
  - Guard allows normal shrink/replacement, only skips severe drops with ephemeral-only additions
  - 3 new regression tests + existing ingest shrink/subset tests passing
- [ ] Fix D: Cache + Archival Death Spiral (CRITICAL)
  - [ ] Analyze cache invalidation propagation from archival
  - [ ] Design cache-aware archival strategy (defer, batch, or cache-breakpoint-aware)
  - [ ] Implement chosen strategy
  - [ ] Add tests for archival without cache invalidation
  - [ ] Validate cache stability across archival events
- [x] Staff-level review of Phase 4 code (planner, heuristics, rewriter, batch gating)
- [ ] Clean code: remove debug noise, ensure all state tracking is correct
- [ ] Re-run manual test Prompts 1+2
- [ ] Verify budget % within 5% of Claude Code's /context report
- [ ] Verify archival reduces context WITHOUT cache death spiral

### Issues Investigated
- [x] 400 "tool use concurrency" — **WAS an Aperture bug** (orphan tool_use after cleanup/archival). Fixed by Fix C.
- [x] Proxy status timeout — transient, MCP binary 30s timeout
- [x] Archival pipeline — verified correct (heuristics→applicator→rewriter→engine), blocked by 400s

## P1: Economics Ledger
- [ ] Add `SessionEconomics` struct tracking all overhead/savings categories
- [ ] Instrument schema injection token counting at injection point
- [ ] Instrument manifest/breadcrumb token counting at injection point
- [ ] Instrument tool response token counting at dispatch point
- [ ] Instrument re-invocation counting and prefix size estimation
- [ ] Track provider-reported usage from API response `usage` fields
- [ ] Track per-request archival savings (archived_tokens accrued over subsequent requests)
- [ ] Expose `/_aperture/economics` endpoint for inspection
- [ ] Add IPC command for frontend economics display
- [ ] Add economics ledger accuracy tests with synthetic scenarios

## P1: Cache-Stable Request Construction (ROOT CAUSE FIX — IMPLEMENTED)
- [x] Remove manifest injection from system message entirely
- [x] Option B selected: Remove manifest entirely (MCP tools provide same info on demand)
- [x] Move breadcrumb to last user message (cache-safe position)
- [x] Add threshold-triggered budget warning (only on alert level escalation)
- [x] Gate heuristic mutations on batch points (task boundary, alert change, explicit commit)
- [x] Add regression tests for cache-stable construction (7 trailing context tests)
- [x] Add batch-point gating tests (5 new planner tests)
- [ ] Add cache_control preservation awareness to rewriter (don't strip existing markers)
- [ ] Protect/relocate cache breakpoints when archived block carries `cache_control` marker
- [ ] For non-MCP paths (Codex): make tool injection idempotent and stable across requests
- [ ] Validate cache stability with controlled A/B test (compare cache_create/cache_read)

## Manual Re-Test Focus (2026-02-19)
- [ ] Verify persistent archive intent behavior: one transition miss, then stable prefix cache hits when same archive set is re-applied
- [ ] Verify no archive-set oscillation across consecutive turns in stateless clients
- [ ] Check whether archived candidates ever include breakpoint-carrying blocks (`cache_control` risk)
- [ ] Verify session affinity stability: consecutive turns in one Claude conversation should reuse the same Aperture session identity (no per-turn session churn)
- [x] Analyze latest mixed-outcome manual run log and classify remaining failures by root cause: old binary/build vs active logic bug vs expected `/context` divergence
- [x] Analyze follow-up manual log (`df4ad515...`, phrase "whats up claude") with line-level timeline for preview/plan(stage)/plan(commit), commit queue behavior, and block/token movement
- [x] Analyze newest follow-up log (`1baf6b88...`, phrase "Yoo claude") with line-level timeline for status/preview/plan(stage)/plan(commit), commit queue behavior, and token movement
- [x] Analyze fresh repro log (`66dd683a...`, phrase "claude!") and correlate JSONL timeline with current Aperture DB state
- [x] Run independent forensic round #3 review and record agreements/disagreements with round-2 findings (`deep-dive-diagnostics-round-3-2026-02-19.md`)
- [x] Add diagnostics-only replay tests for: projection-vs-applied archival mismatch, namespaced MCP cleanup miss, and auxiliary-session active flip behavior
- [x] Add frontend diagnostics test proving session replacement currently produces archival toast false positives
- [x] Hardening for local startup workflow: `~/.config/fish/functions/aperture.fish` `start` now kills stale dev/MCP processes before launch
- [x] Fix persistent archival mutation matching so queued archive actions survive block-ID churn across ingest turns
- [x] Normalize `context_plan(stage)` archive IDs to accept optional `#` prefix from preview output
- [x] Reproduce and fix temporary block disappear/reappear behavior during tool-use subrequests
- [x] Re-run manual test prompts after the above fixes and capture a new log for verification
  - Log: `db654aac-a155-4291-aae6-2cb1dfd20b31` ("whats poppin claude", Opus 4.6)
  - Report: `deep-dive-diagnostics-round-5-2026-02-19.md`

## Round 8 Fixes (2026-02-19) — COMPLETE + VERIFIED R9

### R8-1: Thinking Block Corruption (3 mechanisms)
- [x] F2: Guard thinking/redacted_thinking in `replace_content_block_with_stub()` (payload.rs) — **VERIFIED R9**
- [x] F3: Exclude `Role::Thinking` from archival candidates (heuristics.rs) + validation rejection (validation.rs) — **VERIFIED R9**
- [x] F1: Reorder pipeline: stubs → replacements → removal (payload.rs, all 3 API formats) — **VERIFIED R9**
- [x] F4: Skip merge for consecutive assistant messages with thinking blocks (sanitize.rs) — **VERIFIED R9**

### R8-3: Search broken for multi-word queries
- [x] F6: Tokenize search queries per-term in `search_score()` + `extract_search_snippet()` (tools.rs) — DONE (R9: search endpoint errored, untestable)

### R8-2: Haiku plan malformed input
- [x] F5: Detect unknown plan parameters, return helpful error (plan.rs) — **VERIFIED R9**

### R7-3: Context tool block accumulation
- [x] Fix B: Filter context tool blocks from engine ingest (ingest.rs) — **VERIFIED R9**

### Test counts
- 640 Rust tests passing (605 lib + 4 session + 21 proxy + 10 lifecycle)
- 53 frontend tests passing
- Clippy clean

## Round 9 Verification (2026-02-19) — COMPLETE

**Report**: `deep-dive-diagnostics-round-9-2026-02-19.md`
**Log**: `5a933896-82ba-4604-8e76-4caa21ca16f2.jsonl` (Sonnet 4.6, 87 turns, 88.1% cache hit)

### Results: Zero 400s, all R8 fixes PASS
- [x] Verify zero HTTP 400 errors — **PASS** (0 errors across 87 turns)
- [x] Verify thinking blocks preserved — **PASS** (27 blocks, zero corruption)
- [x] Verify thinking block validation rejects archival — **PASS** (L164: 3 blocks rejected)
- [x] Verify plan param error detection — **PASS** (L58: helpful error, self-corrected)
- [x] Verify context tool block cleanup — **PASS** (no accumulation)
- [x] Verify first archival works — **PASS** (8 blocks, 48%→28%)
- [ ] Verify search tokenization — INCONCLUSIVE (endpoint connection error in R9; search confirmed working in R10)

### New Bugs Found

- [x] **R9-1/MT-1 (P0)**: Plan layering failure — `persistent_archived_ids` not updated at commit time
  - Second/third committed plans never fire, only first plan's 8-block archival persists
  - Root cause: `commit_staged_plan_for_session()` only sets `pending_plan`, does NOT update `persistent_archived_ids`
  - **FIX IMPLEMENTED (2026-02-20)**: Option B — `add_persistent_archives_for_session()` called at commit time
  - **Manual test R10 PASSED**: 3 archive rounds stacked correctly (8+8+5 = 21 blocks)
  - Runtime root cause (H1 vs H2) still needs diagnostic log analysis
- [ ] **R9-2 (P1)**: Session crash on Edit — Edit on RESUME.md triggered session reset
  - 7 rapid file-history-snapshots, terminal white flash, synthetic "No response requested"
  - Catastrophic cache miss: 6.4% hit, 150k tokens re-cached (~$0.94)
  - Edit DID succeed despite crash
  - May be Claude Code file-watcher issue, not Aperture
- [x] **R9-3 (P2)**: Search endpoint connection error
  - **FIX IMPLEMENTED (2026-02-20)**: `call_proxy()` retry (2 attempts, 500ms delay)
  - **Manual test R10**: Search confirmed working

## Round 10 Manual Test (2026-02-20) — PASSED

**Best run yet.** 2 successful context cleans. All fixes applied before test.

### Results
- [x] All 5 MCP tools confirmed working (preview, status, search, read, plan)
- [x] All 6 plan operations confirmed (archive, compress, expand, recall, pin, shift_to)
- [x] Persistent archival stacking: 3 rounds accumulated (8+8+5 = 21 blocks)
- [x] Plans layered with user turns fire correctly
- [ ] Analyze diagnostic logs for H1 vs H2 root cause
- [ ] Fix breadcrumb delta bug (shows +0 for persistent re-archival)
- [ ] Fix budget % gap (~17.5% overhead not included in engine calculation)
- [ ] Investigate Claude Code crash on file edit through proxy (edits land, session dies, R9-2 related)

## Round 5 Bugs — Deep-Dive Diagnostics (2026-02-19)

### BUG #1 (P0): Prefill Error — Payload Ends with Assistant Message
- [x] Trace cleanup → sanitize pipeline ordering in `proxy/rewriter.rs`
- [x] Verify: does `cleanup_history()` strip tool_result but leave tool_use?
  - **Finding**: Cleanup strips BOTH tool_use and tool_result. The issue is that user messages become empty after tool_result removal → deleted → consecutive assistants + assistant-at-end.
- [x] Verify: does `sanitize_anthropic_orphan_tool_uses()` run AFTER cleanup?
  - **Finding**: Yes, correct order. But sanitizer doesn't help — tool_uses already stripped. Problem is structural (missing user messages), not orphan tools.
- [x] Root cause confirmed (Round 6): `strip_anthropic_context_tools()` removes user messages → invalid structure
- [x] Implement fix: add `sanitize_anthropic_message_structure()` in `rewriter/sanitize.rs`
  - Merge consecutive same-role messages
  - Ensure last message is role=user (append synthetic user message if needed)
  - Wire at `rewriter.rs:224` after orphan sanitizers
- [x] Add regression tests (7 new tests in `rewriter/tests.rs`)
- [ ] Verify fix in manual test

### BUG #2 (P1): Trailing Whitespace in Rewritten Assistant Content
- [x] Root cause confirmed (Round 6): Consequence of BUG #1. Cleanup leaves assistant-at-end → Anthropic validates as prefill → finds trailing whitespace in model's natural output.
- [x] Fixed by BUG #1 fix (no separate implementation needed)
- [ ] Verify fix in manual test

### BUG #3 (P1): Thinking Block Modification
- [x] Check if rewriter has guard to skip `thinking`/`redacted_thinking` → No explicit guard needed
- [x] Check if ANSI stripping touches thinking blocks → No (engine-level only, doesn't affect JSON payload)
- [x] Check if stub application iterates over thinking blocks → No (stubs target text/tool_use/tool_result types)
- [x] Root cause confirmed (Round 6): `serde_json = "1"` without `preserve_order` uses BTreeMap → keys sorted alphabetically during `from_slice→to_vec` round-trip. Thinking block keys `{type,thinking,signature}` become `{signature,thinking,type}`. Anthropic's integrity check fails.
- [x] Verified with test program: 10/10 cases fail without `preserve_order`, 9/10 pass with it (only `\/`→`/` edge case remains)
- [x] Implement fix: change `Cargo.toml:20` to `serde_json = { version = "1", features = ["preserve_order"] }`
- [x] Run `cargo test` + `cargo clippy` to verify no regressions (632 Rust tests, clippy clean)
- [ ] Verify fix in manual test

### BUG #5 (P1): Cache Catastrophe on Multi-Tool Turns
- [x] Root cause confirmed (Round 6): System prompt starts with `x-anthropic-billing-header:` (changes per request). `content_fingerprint()` hashes first 200 chars including this header → different block ID each request.
- [x] Evidence: L96 `#e4916327`, L100 `#a5417238`, L104 `#992504a2`, L111 `#a13b421c` — four different system block IDs
- [x] Existing code already knows: `normalize_regression_content()` in ingest.rs filters billing headers. Parser fingerprint does not.
- [x] Implement fix: filter `x-anthropic-billing-header:` lines from system content before `content_fingerprint()` in `parser/anthropic.rs`
- [x] Add regression tests (3 new tests in `parser/tests.rs`)
- [ ] Verify fix in manual test

### BUG #6 (P2): Block Count Inflation
- [x] Partially explained by BUG #5: system block ID churn causes session replacement each ingest
- [ ] Address remaining causes if practical after #5 fix

### Known Limitation: CC's `/context` ≠ Aperture Budget
- [ ] Document as known limitation in user-facing docs/tool responses
- [ ] Consider adding note to `aperture_context_status` response explaining the difference

## Targeted Experiments (2026-02-19) — COMPLETE

### CRITICAL-2 Fix: DONE
- [x] Fixed `is_context_tool_name()` in `runtime.rs` to match `mcp__aperture__aperture_context_*`
- [x] Added `MCP_CONTEXT_TOOL_PREFIX` constant
- [x] Converted 3 replay tests from "bug exists" to "bug is fixed" (Anthropic + OpenAI Chat + OpenAI Responses)
- [x] Extended `test_is_context_tool_name` with MCP-namespaced assertions + false-positive guards
- [x] Full suite: 571 lib + 4 session + 21 proxy + 10 lifecycle tests passing, clippy clean

### Experiment 1: API Content-Block Removal (docs-validated, no direct API test — no API key)
- [x] Documented API constraints: every tool_use needs tool_result, non-empty content, turn alternation
- [x] Option A viable for Anthropic (orphan sanitizer handles cascading), needs new sanitizer for OpenAI
- [x] Option B (stubs) preserves all invariants with zero structural risk
- **Decision: Option B selected**

### Experiment 2: Orphan Sanitizer Interaction — COMPLETE
- [x] Anthropic both directions covered: `sanitize_anthropic_orphan_tool_uses` + `sanitize_anthropic_orphan_tool_results`
- [x] Sanitizers run LAST in pipeline (after all mutations) — correct ordering
- [x] OpenAI: NO orphan sanitizer for non-context pairs — would need new code for Option A
- [x] Conclusion: Option B avoids need for any new sanitizer code

### Experiment 3: Cache Economics — COMPLETE
- [x] Break-even: ~5 requests for either option
- [x] Over 20 requests: $0.46 net savings vs broken state
- [x] Option A vs B: negligible difference ($0.004 over 40 requests, ~100 token stub overhead)
- [x] Current broken state ADDS $0.21+/session (cascading failure)
- [x] Total swing: ~$0.67/session

### Experiment 4: Session Flip Frequency — COMPLETE
- [x] Haiku traffic DOES route through proxy (confirmed: 68/454 sessions are Haiku)
- [x] Flips on nearly every user message (52 opus→haiku transitions)
- [x] Cosmetic only: false toasts, brief UI flicker, self-healing <1s
- [x] **Downgraded to MEDIUM**. Fix: model-aware session creation (~5 lines)

### Decision Summary
- **CRITICAL-1 fix**: Option B (content replacement with stubs) — preserves structure, identical cache economics, works for all API formats
- **HIGH-1 → MEDIUM-1**: Session flips confirmed real but cosmetic. Fix in same batch.

## CRITICAL-1 + MEDIUM-1 Fixes (2026-02-19) — COMPLETE

### CRITICAL-1: Turn-Aware Projection + Partial-Turn Stubs
- [x] Fixed `estimate_token_delta()` in `validation.rs` to be turn-aware:
  - Full-turn archives (all blocks at turn archived): full token savings
  - Partial-turn archives: savings = block tokens - 10 token stub overhead per block
  - Mirrors applicator's actual behavior (stubs for partial, removal for full)
- [x] Updated 2 replay tests to expect corrected projections (-60572 and -46069 vs old -60642/-46139)
- [x] Added 4 new projection tests: full-turn, partial-turn, mixed, archive+compress
- [x] Added 4 new end-to-end rewriter tests: Anthropic tool_use stub, OpenAI Chat stub, OpenAI Responses stub, full pipeline (applicator→payload)
- [x] Partial-turn stub infrastructure confirmed working for all 3 API formats
- [x] Full suite: 587 lib + 4 session + 21 proxy + 10 lifecycle + 53 frontend = 675 tests, clippy clean

### MEDIUM-1: Session Flip Guard
- [x] Fixed `ensure_session()` in `engine/mod.rs` — model-aware guard on `switch_to()`:
  - Only flips active session if same provider+model OR active session is small (<1000 tokens)
  - Prevents Haiku classifier traffic from stealing active status from main Opus conversation
  - Complements existing guard in `SessionStore::create()` (which only covered new session creation)
- [x] Added 3 tests: auxiliary model blocked, same model allowed, small session allows flip
- [x] All 622 Rust tests passing

## Symptom → Root Cause Mapping (Round 4 Consolidated)

| User-Visible Symptom | Root Cause(s) | Fix |
|----------------------|---------------|-----|
| "Archival doesn't work" (commit, /context doesn't drop) | CRITICAL-1 (partial-turn no-op) + CRITICAL-2 (tool calls compound) | P2 + P1 |
| "Blocks disappear and reappear" | HIGH-1 (session flips) + CRITICAL-1 (engine/API divergence → ingest re-creates) | P3 + P2 |
| "Token counts don't match" | HIGH-2 (3 counting domains, structural) | P4 (framing) |
| "Context fills fast" | CRITICAL-2 (tool calls not cleaned) + normal (large file reads) | P1 |

## Refactor-First Track (2026-02-19 pivot)
- [x] Staff-level architecture/code-health review of hot paths:
  - [x] `src-tauri/src/engine/*`
  - [x] `src-tauri/src/proxy/*`
  - [x] `src-tauri/src/metacog/*`
  - [x] `src-tauri/src/bin/aperture_mcp.rs`
- [x] Tranche #3 backend expansion review (beyond tranche #2):
  - [x] `src-tauri/src/proxy/handler.rs` boundary and responsibility split review
  - [x] `src-tauri/src/proxy/interceptor.rs` boundary and responsibility split review
  - [x] `src-tauri/src/proxy/capture.rs` lifecycle/state/telemetry separation review
  - [x] `src-tauri/src/proxy/context_api.rs` route/dispatch/guardrail separation review
  - [x] engine satellite modules/shared helpers for stale pathways and ownership clarity (no additional safe removals required in this tranche)
- [x] Create a refactor map with target boundaries and file split plan (what moves where, why).
- [x] Refresh refactor map for tranche #3 architecture/repo-organization scope.
- [x] Separate inline Rust tests from high-churn runtime files where practical:
  - [x] move large parser `#[cfg(test)]` block into `src-tauri/src/proxy/parser/tests.rs`,
  - [x] move additional large `#[cfg(test)]` blocks from other hotspots (`proxy/rewriter`, `engine/mod`, `engine/planner/mod`, `metacog/tools`),
  - [x] move additional tranche #3 inline tests to `proxy/handler/tests.rs`, `proxy/interceptor/tests.rs`, `proxy/capture/tests.rs`, and `mcp/tests.rs`,
  - [x] keep behavior and coverage equivalent.
- [x] Break up oversized mixed-concern files into focused modules.
  - [x] Parser tranche #1: split `src-tauri/src/proxy/parser.rs` into `src-tauri/src/proxy/parser/{mod,anthropic,openai,identity,overhead,tests}.rs`
  - [x] Follow-on tranche #2: split `rewriter`/`engine`/`planner` hotspots + metacog plan flow
- [x] Tranche #3: split `handler`/`interceptor`/`capture` by ownership boundary and move MCP runtime out of bin entrypoint.
- [x] Post-tranche hygiene: split `context_api` tests out of runtime file and isolate argument/response helper boundaries.
- [x] Remove dead code / duplicate pathways / stale helpers found during review (safe stale pathway cleanup completed for MCP bin/runtime split).
- [x] Standardize core code quality conventions in touched areas:
  - [x] naming consistency,
  - [x] error/context propagation style,
  - [x] logging style and signal-to-noise.
- [x] Repo/docs hygiene tranche:
  - [x] refresh architecture/ownership docs for post-tranche backend structure,
  - [x] tighten phase docs for explicit bug-dive prerequisites and success criteria,
  - [x] add canonical docs navigation map (`docs/DOCS_INDEX.md`) and repo map (`docs/REPO_STRUCTURE.md`),
  - [x] add folder-level indexes (`dev/active/README.md`, `.context/README.md`) to reduce fresh-context navigation cost,
  - [x] create paste-ready post-clear kickoff prompt (`.context/tranche-3-kickoff-prompt.md`).
  - [x] archive completed tranche kickoff prompt under `.context/archive/` to keep one clear fresh-context path.
- [x] Hackathon polish docs tranche:
  - [x] add docs lifecycle governance doc (`docs/DOC_LIFECYCLE.md`),
  - [x] add submission-facing snapshot doc (`docs/HACKATHON_SUBMISSION.md`),
  - [x] add archive indexes (`docs/archive/README.md`, `.context/archive/README.md`),
  - [x] archive stale whimsical `.context` notes under `.context/archive/`,
  - [x] create final post-clear prompt for bug-dive finish (`.context/final-hackathon-polish-prompt.md`).
- [x] Validate after each tranche:
  - [x] tranche #1: `cargo test --manifest-path src-tauri/Cargo.toml`,
  - [x] tranche #1: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`,
  - [x] tranche #1: relevant frontend checks if touched (N/A, frontend untouched).
  - [x] tranche #2: `cargo test --manifest-path src-tauri/Cargo.toml`,
  - [x] tranche #2: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`,
  - [x] tranche #2: relevant frontend checks if touched (N/A, frontend untouched).
  - [x] tranche #3: `cargo test --manifest-path src-tauri/Cargo.toml`,
  - [x] tranche #3: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`,
  - [x] tranche #3: relevant frontend checks only if touched (N/A, frontend untouched).
- [x] Update docs with architecture map + completed cleanup tranche details.
- [x] After cleanup tranche #3, re-open bug-fix track (persistent archival, mismatch, oscillation) from cleaner baseline.

### Findings Backlog (Severity-Ordered)
- [x] `RESOLVED (HIGH)`: split `src-tauri/src/proxy/rewriter.rs` by concern (signal extraction vs sanitization/injection vs provider JSON patching).
- [x] `RESOLVED (HIGH)`: split `src-tauri/src/engine/mod.rs` into ingest/session orchestration vs mutation/event helper modules; extracted large inline tests.
- [x] `RESOLVED (HIGH)`: split `src-tauri/src/engine/planner/mod.rs` validation/projection concerns and extracted large inline tests.
- [x] `RESOLVED (MEDIUM)`: reduced `src-tauri/src/metacog/tools.rs` mixed responsibilities by extracting plan-control/normalization flow.
- [x] `RESOLVED`: `src-tauri/src/proxy/parser.rs` split and inline parser tests moved to `src-tauri/src/proxy/parser/tests.rs`.
- [x] `RESOLVED (HIGH)`: split `src-tauri/src/proxy/handler.rs`, `src-tauri/src/proxy/interceptor.rs`, and `src-tauri/src/proxy/capture.rs` into focused runtime + helper modules with dedicated test files.
- [x] `RESOLVED (MEDIUM)`: split `src-tauri/src/proxy/context_api.rs` inline tests into `src-tauri/src/proxy/context_api/tests.rs` and tightened routing/argument parsing helper boundaries.
- [x] `RESOLVED (MEDIUM)`: moved MCP orchestration from `src-tauri/src/bin/aperture_mcp.rs` into `src-tauri/src/mcp/server.rs` with dedicated tests.

## P3: Schema Overhead Reduction
- [ ] Measure exact token cost of current 5-tool schema injection
- [ ] Evaluate: consolidate 5 tools → 1 unified tool with command parameter
- [ ] Evaluate: consolidate 5 tools → 2-3 essential tools (read + plan + combined preview/status/search)
- [ ] Add lazy injection gate: only inject when budget > 30% AND session established
- [ ] Add progressive injection: start with minimal tools, expand on first use
- [ ] Measure before/after token reduction
- [ ] Tests: tool dispatch through consolidated schema

## P4: Proactive Injection (Eliminate Re-invocations)
- [ ] Design compact proactive context summary format (~50 tokens)
- [ ] Move preview/status to proactive system message injection (no tool call needed)
- [ ] Only inject proactive summary when budget > 40% (meaningful context to manage)
- [ ] Remove preview/status from tool schema (keep read/search/plan as tools)
- [ ] Ensure proactive content is in cache-stable position
- [ ] Tests: proactive injection content, reduced re-invocations

## P5: Delta Protocol
- [ ] Add per-session monotonic revision counter (context_rev)
- [ ] Add per-block change metadata (created_at_rev, modified_at_rev, deleted_at_rev)
- [ ] Add server-side last_served_rev tracking
- [ ] Add since_rev parameter to remaining tool APIs
- [ ] Backward compatible: omit since_rev = full response
- [ ] Tests: delta correctness, tombstone handling, rev ordering

## P6: ROI Controller
- [ ] Implement rolling-window ROI calculation (last 10 requests)
- [ ] Define degradation tiers:
  - ROI > 0: full active mode
  - ROI -10% to 0: suppress proactive injection, keep tools
  - ROI < -10% for 5+ requests: disable tool injection
  - ROI < -25% for 10+ requests: auto-passive
- [ ] Emit operator telemetry for each mode transition
- [ ] Add deterministic tests for all degradation transitions and recovery

## P7: Benchmark Suite (Decision Gate)
- [ ] Define 5-8 representative tasks across categories:
  - Trivial (2-5 requests): fix typo
  - Short (5-15 requests): add function + test
  - Medium (15-40 requests): refactor module
  - Long (40-100+ requests): multi-file feature
  - Tool-heavy (20-60 requests): debug with many greps
- [ ] Build automated harness: run task in passive baseline vs active mode
- [ ] Record: request count, total tokens, provider tokens, economics ledger, wall-clock time
- [ ] Each task run 3-5 times for statistical significance
- [ ] Pass criteria: median active <= median passive per category
- [ ] Generate comparison report with methodology and raw data

## Pivot Gate: Token-Economics Parity
- [ ] All P7 benchmark categories pass median parity test
- [ ] No category where p95 active > 1.1× p95 passive
- [ ] Trivial tasks: overhead < 5000 tokens total
- [ ] Publish parity report
- [ ] Get sign-off to resume expansion

## Paused Expansion Track (DO NOT START until pivot gate passes)
- [ ] Implement real Anthropic compression adapter
- [ ] Implement real OpenAI compression adapter
- [ ] Implement optional OpenRouter adapter
- [ ] Add queue worker execution loop
- [ ] Integration tests for provider timeout/error fail-open
- [ ] Convert autonomous archival/compression to queue jobs
- [ ] Enforce preserve-keys prompt contract
- [ ] Engine apply path for sidekick summaries
- [ ] Compression quality scoring and rejection thresholds
- [ ] Queue/sidekick status in UI

## Verification / Triage (from previous phase)
- [x] Reproduce orphan tool_result MCP smoke-test error
- [x] Add orphan tool_result sanitizer
- [x] Prevent autonomous archival of tool lifecycle blocks
- [x] Stabilize parser block IDs
- [x] Add operator reset path (engine_clear_context)
- [x] Add staged planning controls
- [x] Suppress autonomous heuristics during staged planning
- [ ] Confirm tool lifecycle pairing in manual live run

## Critical Incident Diagnostics (COMPLETE)
- [x] Quantify deduped request/token usage from local Claude session logs
- [x] Confirm large-burn sessions dominated by cache churn and high request counts
- [x] Verify largest-burn sessions not driven by aperture_context_* calls
- [x] Confirm cold-start orphan tool_result fix with regression tests
- [x] Identify trigger path for request fan-out
- [x] Implement guardrails (rate warnings, circuit breaker, size caps, kill switch)
