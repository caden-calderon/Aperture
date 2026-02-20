# Deep Dive Diagnostics Round 2 (2026-02-19)

## Scope
- Mode: diagnostics-only (no production fixes).
- Repro artifacts:
  - Claude log: `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl`
  - Aperture DB: `~/.aperture/aperture.db`
- Added diagnostic replay tests only:
  - `src-tauri/src/engine/planner/tests.rs:602`
  - `src-tauri/src/engine/planner/cleanup.rs:583`
  - `src-tauri/src/engine/tests.rs:484`
  - `src/lib/stores/context-budget.test.ts:169`

## Findings (Severity-Ordered)

### P0: Plan projection can claim `-61k` while rewrite removes zero payload turns
- Confidence: High
- Proof status: Proven (runtime + DB + replay test)
- Evidence:
  - Repro timeline:
    - Preview reports `62% (123k/200k)` at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:71`.
    - Stage/commit report projected `-61k` at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:76` and `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:81`.
    - Assistant claims 7 blocks stripped at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:84` and `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:93`.
    - Claude `/context` rises `64% -> 67% -> 69%` at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:62`, `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:88`, `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:97`.
  - DB correlation (session `151c97a1-cf86-497b-9b58-dac5433d1429`): archived targets covered only part of each turn (`turn 5: 4/6`, `turn 7: 3/4`), so full-turn removal criterion is not met.
  - Applicator contract requires full turn coverage for removal: `src-tauri/src/engine/planner/applicator.rs:190`.
  - Replay test proves mismatch:
    - projection `token_delta = -60,642`,
    - `remove_turns` remains empty,
    - `has_payload_changes == false`.
    - `src-tauri/src/engine/planner/tests.rs:602`

### P0: Context cleanup misses namespaced MCP tool names
- Confidence: High
- Proof status: Proven
- Evidence:
  - Repro uses namespaced tool names:
    - `mcp__aperture__aperture_context_preview` at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:68`
    - `mcp__aperture__aperture_context_plan` at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:73` and `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:78`
  - Matcher is canonical-prefix only: `starts_with("aperture_context_")` in `src-tauri/src/metacog/runtime.rs:57`.
  - Anthropic cleanup path depends on that matcher: `src-tauri/src/engine/planner/cleanup.rs:108`.
  - New diagnostic test confirms no stripping for namespaced calls: `src-tauri/src/engine/planner/cleanup.rs:583`.

### P1: Active session flips are inherent when auxiliary sessions ingest, which can produce false archival UI notifications
- Confidence: High (mechanism), Medium-High (frequency impact in live repro)
- Proof status: Mechanism proven; user-visible frequency still needs live event trace
- Evidence:
  - DB shows interleaved side sessions (Haiku topic-classifier traffic) during same repro window with `isNewTopic` payloads.
  - Session creation always sets new active session: `src-tauri/src/engine/session.rs:115`.
  - UI refresh reads active session blocks and pushes them directly to store: `src/routes/+page.svelte:69`.
  - Store compares old/new IDs and toasts missing IDs as archived without session-switch guard: `src/lib/stores/context.svelte.ts:861`.
  - Engine replay test proves active flips to auxiliary session and back only after primary re-ingest: `src-tauri/src/engine/tests.rs:484`.
  - Frontend test proves unrelated session replacement is interpreted as archival toast: `src/lib/stores/context-budget.test.ts:169`.

### P1: Token bar vs Aperture tools vs Claude `/context` mismatch remains structural
- Confidence: High
- Proof status: Proven (design mismatch, not a single-counter bug)
- Evidence:
  - Backend budget includes overhead tokens: `src-tauri/src/engine/mod.rs:184`.
  - UI budget derives from block sum only via `calculateTokenBudget`: `src/lib/stores/context.svelte.ts:248`, `src/lib/mock-data.ts:612`.
  - Repro shows divergence in same window:
    - `/context` `64/67/69%` at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:62`, `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:88`, `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:97`
    - Aperture preview `62%` at `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl:71`

## What Was Ruled Out
- "Stale `/tmp` artifacts caused this round's conclusions."
  - Ruled out: all evidence came from fresh latest repro log (`66dd...`, mtime Feb 19 12:38 local) and current DB snapshot.
- "Cleanup mismatch is only hypothetical."
  - Ruled out via failing behavior replay test (`cleanup.rs:583`) and live namespaced tool names in repro log.
- "Projection vs no-drop is only due to metric-domain differences."
  - Ruled out: replay test isolates applicator behavior and shows zero payload removal despite large projected delta.

## Open Questions
- Does persistent archive intent later compensate for partial-turn archive sets in some sessions, or does this mismatch persist indefinitely under tool-heavy turns?
- How often do auxiliary-session ingests occur during normal workflows (vs manual stress tests), and what is the exact UI oscillation cadence in event stream terms?
- Should projection semantics be rewritten to report payload-feasible savings rather than block-sum savings, or should UX language be split into "engine archive intent" vs "payload removal applied"?

## Proof Status
- Proven:
  - Projection can materially overstate real payload removal.
  - Namespaced MCP tool names are missed by cleanup matcher.
  - Active-session flips happen on auxiliary ingest and can map to false archival toasts.
  - Token metric divergence is structural across counting domains.
- Suspected / needs more evidence:
  - Exact contribution of cleanup mismatch to observed disappear/reappear amplitude in long tool chains.
  - Exact rate of active-session churn in production-like usage without manual stress prompts.

## Targeted Validation Run
- Rust:
  - `cargo test --manifest-path src-tauri/Cargo.toml replay_projection_overstates_payload_savings`
  - `cargo test --manifest-path src-tauri/Cargo.toml namespaced_mcp_context_tools_are_not_matched_currently`
  - `cargo test --manifest-path src-tauri/Cargo.toml auxiliary_session_flips_active_session`
- Frontend:
  - `npx vitest run src/lib/stores/context-budget.test.ts`

## Next Diagnostic Experiments
1. Add a structured event-trace harness for one repro run:
   - emit `(timestamp, resolved_session_id, active_session_id, source, thread_identity, block_count, tokens, event_type)` on ingest and UI refresh.
2. Build a replay test that feeds real `66dd...` block turn layout into planner+rewriter end-to-end and measures actual JSON payload delta bytes/tokens vs projected delta.
3. Add cleanup replay fixtures with real namespaced MCP call/result chains across Anthropic + OpenAI payload shapes.
4. Run the required docs research round:
   - Claude Code `/context` accounting and MCP behavior,
   - Anthropic caching + `cache_control` invalidation boundaries,
   - Codex/OpenAI tool-history semantics and token accounting differences.
