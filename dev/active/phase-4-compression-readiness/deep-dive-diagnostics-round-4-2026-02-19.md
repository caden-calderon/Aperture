# Deep Dive Diagnostics Round 4 (2026-02-19)

## Scope
- Mode: diagnostics-only (no production fixes).
- Prompt anchor: `.context/deep-dive-diagnostics-round-3-prompt.md`.
- Fresh repro anchor (this round): `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl`.
- DB anchor: `~/.aperture/aperture.db`.
- Targeted diagnostics tests added and run; no runtime logic changes.

## Findings (Severity-Ordered)

### CRITICAL-1: Fresh repro still shows large projected archival savings with no observable Claude payload reduction
- Confidence: High
- Proof status: Proven (fresh runtime + code path + replay tests)

Evidence:
- Fresh run timeline (`a24...`):
  - `/context` baseline `152k/200k (76%)` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:58`.
  - Preview tool called at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:64`; result reports `Context Preview — 47 blocks, 72% budget (145k/200k)` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:68`.
  - Plan stage reports `Projected impact: -46k tokens, 48 blocks, 52% budget` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:78`.
  - Commit reports same projection and `Commit queued` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:83`.
  - Subsequent `/context` checks rise to `156k/200k (78%)` and `158k/200k (79%)` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:91` and `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:100`.
- Code path unchanged from prior rounds:
  - Planner projection sums archived blocks (`src-tauri/src/engine/planner/validation.rs:236`).
  - Applicator only marks payload removals when turn-level coverage is complete (`src-tauri/src/engine/planner/applicator.rs:221`).
  - Rewriter returns `Ok(None)` on no payload changes but still applies engine-side archive updates (`src-tauri/src/proxy/rewriter.rs:148`, `src-tauri/src/proxy/rewriter.rs:154`, `src-tauri/src/proxy/rewriter.rs:156`).
- Replay proof:
  - Existing replay still passes (`src-tauri/src/engine/planner/tests.rs:602`).
  - New fresh-run replay added and passing: `src-tauri/src/engine/planner/tests.rs:699` (`-46,139` projected, zero payload removals).

### CRITICAL-2: Namespaced MCP context-tool cleanup miss persists across Anthropic + OpenAI chat + OpenAI responses paths
- Confidence: High
- Proof status: Proven

Evidence:
- Fresh run uses namespaced tool names:
  - `mcp__aperture__aperture_context_preview` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:64`.
  - `mcp__aperture__aperture_context_status` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:65`.
  - `mcp__aperture__aperture_context_plan` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:75` and `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:80`.
- Canonical matcher is still prefix-only (`src-tauri/src/metacog/runtime.rs:60`).
- Cleanup/interception call sites all depend on this matcher (`src-tauri/src/engine/planner/cleanup.rs:108`, `src-tauri/src/engine/planner/cleanup.rs:221`, `src-tauri/src/engine/planner/cleanup.rs:327`, `src-tauri/src/proxy/interceptor/response.rs:59`).
- Diagnostics tests proving non-match now cover all three payload families:
  - Anthropic: `src-tauri/src/engine/planner/cleanup.rs:583`.
  - OpenAI chat: `src-tauri/src/engine/planner/cleanup.rs:733`.
  - OpenAI responses: `src-tauri/src/engine/planner/cleanup.rs:836`.

### HIGH-1: Active-session churn remains structurally capable of causing false archival toasts under auxiliary/session-interleaved traffic
- Confidence: High (mechanism), Medium-High (fresh-run frequency impact)
- Proof status: Mechanism proven; fresh-run cadence corroborated by DB snapshot

Evidence:
- Session creation sets active session immediately (`src-tauri/src/engine/session.rs:134`).
- UI refresh always reads active session blocks (`src/routes/+page.svelte:69`, `src/routes/+page.svelte:71`, `src/routes/+page.svelte:79`).
- Store mutation toast compares old/new IDs and labels missing IDs as archived without session-scope guard (`src/lib/stores/context.svelte.ts:861`, `src/lib/stores/context.svelte.ts:870`, `src/lib/stores/context.svelte.ts:873`).
- Existing replay tests remain valid:
  - engine: `src-tauri/src/engine/tests.rs:484`.
  - frontend toast behavior: `src/lib/stores/context-budget.test.ts:169`.
- Fresh DB window for `a24...` timeframe (`2026-02-19T17:41Z`..`17:45Z`) shows auxiliary Haiku and many short-lived Opus sessions interleaving with main-run sessions, consistent with churn pressure in active-session routing.

### HIGH-2: Token-domain mismatch remains structural and should be framed as domain divergence, not single-counter failure
- Confidence: High
- Proof status: Proven

Evidence:
- In same fresh run window:
  - Aperture preview/status view: `145k/200k (72%)` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:68` and `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:71`.
  - Claude `/context`: `152k -> 156k -> 158k` at `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:58`, `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:91`, `~/.claude/projects/-home-caden-projects-Aperture/a24bae73-cfee-4c06-b429-1e2d223c83c7.jsonl:100`.
- Source-of-truth mismatch in code remains:
  - backend budget path includes overhead domain (`src-tauri/src/engine/mod.rs:184`),
  - UI bar computes from block sums and mutation notifications (`src/lib/stores/context.svelte.ts:850`).

## Research Round (Official Docs/Web Validation)

### Confirmed constraints
1. Claude Code sends full context each request and MCP servers add tool definitions to requests.
   - Source: `https://code.claude.com/docs/en/how-claude-code-works`.
2. Claude MCP tool naming format is namespaced as `mcp__<server>__<tool>`.
   - Source: `https://docs.anthropic.com/en/docs/agents-and-tools/agent-sdk/mcp`.
3. Anthropic prompt caching is prefix/cumulative, with cache hierarchy `tools -> system -> messages`, explicit invalidation boundaries, and bounded lookback per breakpoint.
   - Source: `https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching`.
4. OpenAI Responses API supports stateless usage by default; conversation state can be resumed with `previous_response_id`, and prior history still contributes to billed input tokens.
   - Source: `https://platform.openai.com/docs/guides/conversation-state` and `https://platform.openai.com/docs/api-reference/responses/create`.
5. Codex (OpenAI coding agent) currently operates statelessly in practice.
   - Source: `https://openai.com/index/why-language-models-hallucinate`.

### Aperture comparison
- Aperture cleanup behavior conflicts with confirmed MCP naming convention (critical mismatch with namespaced names).
- Aperture archival projections do not reflect payload-feasible mutation constraints, which conflicts with user-facing expectations in stateless full-history request models.
- Token-domain divergence is expected under both Anthropic and OpenAI semantics because provider-visible usage includes non-block domains.

## What Was Ruled Out
- Stale-log-only diagnosis: ruled out by fresh `a24...` repro and fresh DB correlation in this round.
- Stage/commit dispatch failure as primary cause: ruled out in fresh run (stage + commit both return success with queued apply semantics).
- Single-runtime-only cleanup issue: ruled out by OpenAI chat/responses diagnostics tests added this round.

## Open Questions
1. What is the smallest payload-level mutation strategy (block-level removal vs replacement stub) that preserves turn invariants while making partial archives effective?
2. How much of observed `/context` growth during archival runs is attributable to retained namespaced context tool history versus normal conversation growth?
3. Should session activation policy be source/model-aware (primary-vs-aux) or should UI mutation detection become session-scoped first?
4. Are block IDs sufficiently session-unique under repeated similar content across many concurrent sessions, or is a session-salt needed in ID derivation?

## Proof Status
- Proven:
  - Fresh repro confirms projection/payload mismatch persists (`-46k` projected, `/context` rises).
  - Namespaced MCP cleanup miss persists across Anthropic/OpenAI chat/OpenAI responses.
  - Active-session flip mechanism and false-toast interpretation remain valid.
  - Token-domain divergence remains structural.
- Suspected / needs more evidence:
  - Relative contribution of each issue to end-user perceived regressions in non-stress real workflows.
  - Session-agnostic block-ID collision risk under high parallel-session load.

## Targeted Validation Run (This Round)
- `cargo test --manifest-path src-tauri/Cargo.toml test_replay_projection_overstates_payload_savings_when_archive_set_is_partial_per_turn`
- `cargo test --manifest-path src-tauri/Cargo.toml test_replay_projection_overstates_payload_savings_for_fresh_a24_archive_set`
- `cargo test --manifest-path src-tauri/Cargo.toml namespaced_mcp_context_tools_are_not_matched_currently`
- `cargo test --manifest-path src-tauri/Cargo.toml auxiliary_session_flips_active_session`
- `npx vitest run src/lib/stores/context-budget.test.ts`

All passed.

## Next Diagnostic Experiments
1. Add an end-to-end replay harness that computes actual rewritten request byte/token deltas from captured JSONL request bodies for the exact staged archive set.
2. Add session-scoped event tracing `(resolved_session_id, active_session_id, source, model, block_count)` to correlate churn frequency with UI mutation toasts quantitatively.
3. Add a replay test that injects namespaced MCP context tools in mixed real-tool chains and quantifies retained-token overhead over N turns.
4. Build a three-domain token dashboard fixture (Aperture block sum, Aperture effective, provider `/context`/usage) for one fresh run to standardize diagnostics framing.
