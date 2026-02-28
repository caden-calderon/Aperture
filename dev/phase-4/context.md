# Phase 4 — Working Context

> **For the next agent**: Read this file top to bottom before doing anything else.
> All completed fix history has been archived to `archive-phase4-fixes.md`.

---

## Current State (2026-02-28)

**Version 0.10.0. Session fragmentation and block-wipe bugs FIXED + VERIFIED.**
**Build fingerprinting pipeline operational. Active work: codebase refactor.**

Test counts: 672 Rust + 53 frontend = 725 total. Clippy clean.

### What changed since last update

**Session fragmentation (H9) — FIXED**: `fallback_thread_identity()` in `parser/identity.rs`
produced different hashes per request (Claude Code injects varying `<system-reminder>` content).
Disabled fallback hashing entirely — sessions now key on `(provider, model, source, "default")`.
Confirmed: 1 session instead of 20+, blocks accumulate correctly across turns.

**`/context` block wipe — FIXED**: MCP tool-call bursts (from `/context` slash command) sent
requests where the context-tool filter stripped all but 1 block. Added extreme-collapse guard
in `is_regressive_semantic_collapse`: if old > 4 and new <= 2, always regressive (skip ingest).
Does NOT affect Aperture tools (plan/clear/remove go through engine directly, not ingest).

**Build fingerprinting**: `build.rs` captures git hash + timestamp. All 3 binaries have
`--version` flag. `/_aperture/version` endpoint. Proxy logs to `/tmp/aperture-proxy.log`.
Fish function: `aperture version` (compare local vs running), `aperture logs` (tail proxy log).
Makefile: `rebuild`, `version`, `dev-full` targets.

**Version bumped to 0.10.0** across Cargo.toml, tauri.conf.json, package.json.

### Known Issues (low severity)

- **Primacy/Middle zone flicker during `/context`**: When the collapse guard skips MCP burst
  ingests, the zone classification doesn't re-run. Primacy and Middle zones empty briefly
  (system prompt block and middle blocks disappear from those zones). All blocks remain in
  Recency. Everything restores on the next real prompt. Cosmetic only — blocks are never lost.
- **Proxy log file empty if proxy started before Tauri rebuild**: The proxy must be spawned by
  the updated Tauri binary to get stderr → `/tmp/aperture-proxy.log`. Use `aperture start` to
  ensure a clean rebuild cycle.
- **Diagnostic file logging active**: `diag()` in `engine/ingest.rs` writes to
  `/tmp/aperture-ingest.log` on every ingest. Keep until stability fully confirmed, then remove.
  Also DIAG `warn!` calls in `proxy/handler/exchange.rs` and `engine/mod.rs`.

For full fix history (plan layering, thread identity, WebKitGTK crash, proxy decoupling),
see `archive-phase4-fixes.md`.

---

## Active Work: Codebase Refactor

**Master plan index**: `dev/phase-4/refactor-plan.md`
**Detailed refactor docs**: `dev/phase-4/refactor/README.md` (overview + split audit/session files)

### Phase Sequence
- **Phase A: Backend (Rust)** — IN PROGRESS (A.0 exploration underway)
- **Phase B: Frontend (Svelte/TS)** — PENDING
- **Phase C: Docs & Project Structure** — PENDING

### Phase A Sub-steps
- **A.0: Exploration** — file-by-file audit, document every `.rs` file in the audit table ← YOU ARE HERE
- **A.1: Test extraction** — inline `#[cfg(test)]` → `tests/` directories, one file per concern
- **A.2: File splitting** — oversized files where splitting improves clarity (not just line count)
- **A.3: Module organization** — files in wrong places, missing groupings
- **A.4: Code quality** — dead code, patterns, bugs, comments

---

## A.0 Exploration — Session Protocol

### Mindset: Staff Engineer Code Review

This is not a skim. This is a **staff-level code review of the entire codebase** — the kind you do before a major refactor when you need to know exactly what you're working with. Every file. Every function. Every design decision.

Read like you're the new senior engineer inheriting this codebase and you need to understand it deeply enough to make confident changes. You are building a complete, accurate picture. The audit table is the artifact — it must be good enough that another engineer could pick it up and execute the refactor without reading the source files themselves.

**What "thorough" means here:**
- Read every function, not just the top-level structure
- Notice what's missing, not just what's present (missing error handling, missing docs, missing tests for edge cases)
- Ask "why is this done this way?" — if you can't answer it from the code, flag it
- Notice coupling: what does this file know about that it shouldn't?
- Notice duplication: have you seen this pattern before in another file?
- Notice asymmetry: are two similar things done differently for no apparent reason?
- For large files: read the whole thing — no skimming past the bottom half

This will not happen again. The goal is to do it once, do it right, and have a complete map.

### Rules (Follow These Exactly)

1. **Pure exploration. Do NOT edit any source files. Do NOT restructure anything.**
2. **Read files in logical groups of ~5** (4–6 max per turn). Never ingest a whole module at once — context window will fill before you can document anything.
3. **Read each file completely.** Don't skim. Don't skip to the bottom. Don't assume a file is simple from its line count — a 150-line file can hide a subtle bug.
4. **After each group: update the appropriate audit table file immediately** while the files are fresh. Don't batch the writing — findings degrade fast.
   - Claude: `dev/phase-4/refactor/audit-claude.md`
   - Codex: `dev/phase-4/refactor/audit-codex.md`
5. **After updating the audit table: synthesize the group out loud.** Write a brief paragraph summarizing what you found — patterns, bugs, smells, design decisions, surprises. This sharpens your own understanding and makes the session handoff useful.
6. **Check `/context` between groups.** If approaching 70%, trigger the end-of-session checklist below before stopping.
7. **Large files (500+ lines) get their own dedicated read.** Don't pair them with others — they need full attention.
8. **Do not re-read historical rows/logs every session.** Use `NEXT` in this file, append new rows, and only revisit old entries when explicitly reconciling a finding.

---

### End-of-Session Checklist (Do This When Context Hits ~70%)

When context is filling up, **stop reading new files** and do this before the user clears:

**1. Finish the current group first.**
If you're mid-batch, finish documenting the files already read. Don't leave partial work.

**2. Update the audit table.**
Every file read this session must have a row in your agent audit file under `dev/phase-4/refactor/`. No gaps.

**3. Add a session log row.**
In `dev/phase-4/refactor/session-log.md` (under your agent — Claude or Codex), add a row for this session with date and files covered.

**4. Update the "Covered" list in this file.**
Move files from "NEXT" to "Covered (do NOT re-read)" in the A.0 Current Progress section below.

**5. Update the "NEXT" list.**
The first file in NEXT should be the actual next unread file — not one already covered.

**6. Update "Key Findings So Far" if anything new was found.**
Any new bugs or smells not yet in that section should be added. Don't duplicate entries already there.

**7. Update `bugs.md` if new bugs were found.**
If you found a new real bug (not just a smell), add it to `~/.claude/projects/-home-caden-projects-Aperture/memory/bugs.md`.
The audit tables remain the source of truth; `bugs.md` is the external bug index.

**8. Tell the user what you did and what's next.**
Brief summary: files covered this session, most interesting finding, exact next file to pick up from.

---

### What to Document Per File (Audit Table Columns)

- **Lines**: Exact count
- **Purpose**: One sentence — what does this file own?
- **Tests**: Inline `#[cfg(test)]`? How many lines? Or none?
- **Issues**: Bugs, smells, dead code, architectural concerns — **see detail standard below**
- **Action Needed**: Concrete next step, specific enough to start writing code from

#### Detail Standard — Issues Column

Vague entries are worthless at execution time. Every issue must include:
- **Where**: file + function name + line number
- **What it does wrong**: the precise mechanism, not a label ("mismatches pairs" not "matching bug")
- **Conditions**: when does this actually matter? what breaks?
- **How to fix**: concrete enough that someone can start implementing without re-reading the file

**Bad**: `"Tool chain matching bug — fix tool_use_id"`
**Good**: `"build_dependencies() line 181 matches ToolUse→ToolResult by tool_name string. If read_file is called twice in one session, find() returns the first ToolResult after each ToolUse's turn — second ToolUse at turn 7 may steal the result already claimed by turn 3. Fix: add tool_use_id: Option<String> to BlockMetadata, populate in parser, match by ID in build_dependencies() with name as fallback."`

#### Uncertain Findings — Log Them, Don't Drop Them

If you spot something that looks wrong but can't confirm it without reading another file:
1. **Do not discard it.** Half-seen bugs are still bugs.
2. Add it to the **Uncertain Findings table** in `dev/phase-4/refactor/uncertain-findings.md` with: what you observed, what file/function would confirm or deny it.
3. Note it in your synthesis paragraph as well.
4. Move on. The reconciliation pass investigates these after all files are read.

### Philosophy
Line counts are guidelines for flagging, not mandates for splitting. A 500-line file that is cohesive stays as-is. A 70-line function doing one clear thing is fine. The question is always: **"Does splitting this actually improve clarity, or just make the number smaller?"** If the answer is the latter, leave it.

---

## A.0 Current Progress

### **ALL 87 FILES COVERED — A.0 EXPLORATION COMPLETE (2026-02-25)**

A.0 is done. Proceed directly to **A.1: Test extraction** (inline `#[cfg(test)]` → `tests/` directories).

**Covered (do NOT re-read):**
All of `engine/`, `engine/planner/`, `proxy/`, `metacog/`, `events/`, `mcp/`, `terminal/`, and crate roots/bins — complete.

- `engine/types.rs`, `engine/block.rs` — pure data enums/structs, clean
- `engine/zone.rs`, `engine/session.rs`, `engine/store.rs` — core data structures
- `engine/budget.rs`, `engine/staleness.rs`, `engine/tokens.rs`, `engine/dependency.rs` — economics/scoring
- `engine/versioning.rs`, `engine/action_log.rs`, `engine/policy.rs`, `engine/pipeline.rs`, `engine/ingest.rs` — operation layer
- `engine/session_sync.rs`, `engine/storage.rs`, `engine/mod.rs`, `engine/tests.rs` — persistence + coordinator + tests
- `engine/compression/mod.rs`, `engine/compression/queue.rs`, `engine/compression/provider.rs` — compression
- `engine/planner/types.rs`, `engine/planner/validation.rs`, `engine/planner/relevance.rs`, `engine/planner/manifest.rs` — planner foundations
- `engine/planner/file_tracker.rs`, `engine/planner/heuristics.rs`, `engine/planner/cleanup.rs`, `engine/planner/applicator.rs`, `engine/planner/mod.rs` — planner core
- `engine/planner/tests.rs` — 2010L planner test suite
- `proxy/mod.rs`, `proxy/error.rs`, `proxy/runaway_guard.rs`, `proxy/hot_patch.rs`, `proxy/provider_adapter.rs`
- `proxy/parser/overhead.rs`, `proxy/parser/identity.rs`, `proxy/parser/mod.rs`, `proxy/parser/anthropic.rs`
- `proxy/parser/openai.rs`, `proxy/parser/tests.rs`
- `proxy/rewriter.rs`, `proxy/rewriter/trailing.rs`, `proxy/rewriter/signals.rs`, `proxy/rewriter/sanitize.rs`, `proxy/rewriter/payload.rs`, `proxy/rewriter/tests.rs`
- `proxy/capture.rs`, `proxy/capture/sse.rs`, `proxy/capture/tests.rs`
- `proxy/handler.rs`, `proxy/handler/exchange.rs`, `proxy/handler/headers.rs`, `proxy/handler/routing.rs`, `proxy/handler/tests.rs`
- `proxy/interceptor.rs`, `proxy/interceptor/response.rs`, `proxy/interceptor/tests.rs`
- `proxy/context_api.rs`, `proxy/context_api/tests.rs`, `proxy/ipc_api.rs`
- `metacog/mod.rs`, `metacog/passive.rs`, `metacog/claude_mcp.rs`, `metacog/preview.rs`
- `metacog/runtime.rs`, `metacog/codex_proxy.rs`, `metacog/tools/plan.rs`, `metacog/tools/tests.rs`, `metacog/tools.rs`
- `events/types.rs`, `events/broadcaster.rs`, `events/dispatcher.rs`, `events/mod.rs`
- `mcp/mod.rs`, `mcp/server.rs`, `mcp/tests.rs`
- `terminal/error.rs`, `terminal/session.rs`, `terminal/mod.rs`, `terminal/codex_bridge.rs`
- `lib.rs`, `util.rs`, `main.rs`, `bin/aperture_mcp.rs`, `bin/aperture_proxy.rs`

### NEXT: Begin A.1 — Test extraction

A.0 is complete. Next phase is **A.1: Extract inline `#[cfg(test)]` blocks to `tests/` directories**.

Target files (all have inline tests that need extraction):
- `engine/`: zone, session, store, budget, staleness, tokens, dependency, versioning, action_log, policy, pipeline, storage, compression/mod, compression/queue, compression/provider, planner/types, planner/relevance, planner/manifest, planner/file_tracker, planner/heuristics, planner/cleanup, planner/applicator → `engine/tests/`
- `proxy/`: mod, runaway_guard, hot_patch + all submodule inline tests → `proxy/tests/`
- `metacog/`: mod, passive, claude_mcp, preview, runtime, codex_proxy → `metacog/tests/`
- `events/broadcaster.rs` → `events/tests/`
- `terminal/session.rs`, `terminal/mod.rs`, `terminal/codex_bridge.rs` → `terminal/tests/`
- `util.rs` → `tests/util_tests.rs`

Large files already properly separated: `engine/tests.rs`, `engine/planner/tests.rs`, `proxy/parser/tests.rs`, `proxy/rewriter/tests.rs`, `proxy/capture/tests.rs`, `proxy/handler/tests.rs`, `proxy/interceptor/tests.rs`, `proxy/context_api/tests.rs`, `mcp/tests.rs`, `metacog/tools/tests.rs` — no extraction needed (already separate).

---

---

## Codex Pass — State

Codex does a second independent pass over the same files in the same order as Claude.
The goal is a double-check: catch things Claude missed, confirm things Claude flagged.

**Codex: read this section to know where you are.**

### Codex Instructions
1. Follow the same session protocol as Claude (groups of ~5, update table immediately, synthesize, check context).
2. Add your findings to `dev/phase-4/refactor/audit-codex.md` — not Claude's audit file.
3. Read `dev/phase-4/refactor/audit-claude.md` for the files you're about to read **after** doing your own analysis, not before. You want an independent take first, then compare.
4. If you confirm a Claude finding, note "Confirmed: [issue]" in your Issues column. If you find something Claude missed, note it fresh.
5. Follow the same end-of-session checklist above, updating the **Codex Sessions** log (not Claude's).
6. **Strict lockstep rule**: Codex must stay behind Claude coverage. Do not read any file that does not already have a Claude row in `dev/phase-4/refactor/audit-claude.md`.
7. If Codex reaches/passes Claude's frontier, **stop reading** and wait. Resume only on files Claude has covered.

### Codex Files Covered: 87 of 87 Claude-covered files (A.0 complete)

**Covered (do NOT re-read):**
- `engine/types.rs`, `engine/block.rs`
- `engine/zone.rs`, `engine/session.rs`, `engine/store.rs`
- `engine/budget.rs`, `engine/staleness.rs`, `engine/tokens.rs`, `engine/dependency.rs`
- `engine/versioning.rs`, `engine/action_log.rs`, `engine/policy.rs`, `engine/pipeline.rs`, `engine/ingest.rs`
- `engine/session_sync.rs`, `engine/storage.rs`
- `engine/mod.rs`, `engine/tests.rs`
- `engine/compression/mod.rs`, `engine/compression/queue.rs`, `engine/compression/provider.rs`
- `engine/planner/types.rs`, `engine/planner/validation.rs`, `engine/planner/relevance.rs`, `engine/planner/manifest.rs`
- `engine/planner/file_tracker.rs`, `engine/planner/heuristics.rs`, `engine/planner/cleanup.rs`, `engine/planner/applicator.rs`, `engine/planner/mod.rs`
- `engine/planner/tests.rs`, `proxy/parser/openai.rs`
- `proxy/mod.rs`, `proxy/error.rs`, `proxy/runaway_guard.rs`, `proxy/hot_patch.rs`, `proxy/provider_adapter.rs`
- `proxy/parser/overhead.rs`, `proxy/parser/identity.rs`, `proxy/parser/mod.rs`, `proxy/parser/anthropic.rs`
- `proxy/parser/tests.rs`
- `proxy/rewriter.rs`, `proxy/rewriter/trailing.rs`, `proxy/rewriter/signals.rs`, `proxy/rewriter/sanitize.rs`, `proxy/rewriter/payload.rs`
- `proxy/rewriter/tests.rs`
- `proxy/capture.rs`, `proxy/capture/sse.rs`, `proxy/capture/tests.rs`
- `proxy/handler.rs`
- `proxy/handler/exchange.rs`, `proxy/handler/headers.rs`, `proxy/handler/routing.rs`, `proxy/handler/tests.rs`
- `proxy/interceptor.rs`, `proxy/interceptor/response.rs`, `proxy/interceptor/tests.rs`
- `proxy/context_api.rs`, `proxy/context_api/tests.rs`, `proxy/ipc_api.rs`
- `metacog/mod.rs`, `metacog/passive.rs`, `metacog/claude_mcp.rs`, `metacog/preview.rs`
- `metacog/runtime.rs`, `metacog/codex_proxy.rs`, `metacog/tools/plan.rs`, `metacog/tools/tests.rs`, `metacog/tools.rs`
- `events/types.rs`, `events/broadcaster.rs`, `events/dispatcher.rs`, `events/mod.rs`
- `mcp/mod.rs`, `mcp/server.rs`, `mcp/tests.rs`
- `terminal/error.rs`, `terminal/session.rs`, `terminal/mod.rs`, `terminal/codex_bridge.rs`
- `lib.rs`, `util.rs`, `main.rs`, `bin/aperture_mcp.rs`, `bin/aperture_proxy.rs`

**NEXT (Codex):**
A.0 is complete for Codex. Begin **A.1 test extraction** in lockstep with the phase plan.

---

## Key Findings So Far (Do NOT Re-Investigate)

Already logged in the audit table. Captured here for continuity.

**Universal pattern**: Every source file with logic has inline `#[cfg(test)]`. Test extraction to `engine/tests/<concern>_tests.rs` is the entire A.1 phase. ~12 files need it.

**Bugs found:**
- `engine/dependency.rs`: Tool chain matching uses tool name, not `tool_use_id` — mismatches when same tool called twice in a session
- `engine/ingest.rs`: `block_semantic_fingerprint()` uses `DefaultHasher` (SipHash, randomly seeded per process) — fingerprints non-deterministic, regression guard may silently fail
- `engine/pipeline.rs`: `recency_boost` in `HeuristicResult` is computed but never read by `classify()` — dead field
- `engine/store.rs`: `replace_all()` claims atomic replacement but performs `clear()` + reinserts, allowing concurrent readers to observe transient empty/partial state
- `engine/session.rs`: `SessionStore::create()` can flip active session on same provider+model even when current active session is substantial (possible unintended session steal in multi-session same-model workflows)
- `engine/storage.rs`: `load_block_ids()` orders only by `turn_index`; blocks sharing a turn can reload in nondeterministic order, changing same-turn fragment ordering
- `engine/mod.rs`: `clear_all_sessions()` clears in-memory state even if DB delete calls fail, allowing state resurrection on restart
- `engine/compression/mod.rs`: `default_backend_for_provider()` misses explicit `"openrouter"` provider name, so Auto can misroute to Anthropic
- `engine/planner/heuristics.rs`: `is_archival_candidate()` docs say Recency is excluded, but code excludes only Primacy; at soft pressure Recency blocks can still be archived
- `engine/planner/applicator.rs`: archival dominance is enforced only for engine updates; in partial-turn archive+compress/update cases, payload can still emit conflicting replacement + stub decisions
- `engine/planner/mod.rs`: threshold warning text reports budget ceiling percent as soft/medium/hard threshold percent, misleading operators under custom ceilings
- `engine/planner/mod.rs`: `commit_staged_plan_for_session()` does not atomically update `persistent_archived_ids` — caller must separately call `add_persistent_archives_for_session()`. Two-step API where omitting either step silently causes regression (R9-1 regresses if step 2 omitted).
- `engine/planner/mod.rs`: R9-DIAG `warn!()` at line 555 is still in production — fires on every pending plan application; should be downgraded to `debug!()` post-regression-resolution
- `proxy/parser/mod.rs` + `proxy/parser/identity.rs`: `stable_block_id()` and `short_hash()` both use `DefaultHasher::new()` — currently deterministic (fixed SipHash initial state) but API contract says "not specified, may change." Rust version upgrade could silently change block IDs and break session continuity + archived block re-application. Fix both with `FxHasher` alongside the `ingest.rs` fix.
- `proxy/parser/anthropic.rs`: billing-header filter logic duplicated in both `anthropic.rs` and `identity.rs` — extract shared helper.
- `proxy/parser/openai.rs`: unknown roles default to `Role::User`; OpenAI `developer` messages are misclassified and lose primacy semantics.
- `proxy/parser/openai.rs` + `engine/block.rs`: tool call IDs (`tool_call_id`/`call_id`) are parsed into content strings but not persisted in block metadata; deterministic ToolUse→ToolResult matching cannot be completed until `tool_use_id` is added to metadata and populated in parsers.
- `proxy/parser/openai.rs`: `parse_openai_responses_request()` missing `"function_call"` input item handler (line 412) — assistant tool calls appearing in Responses API history array fall to unknown branch → wrong role (User instead of ToolUse), wrong content format. Breaks multi-turn Codex session continuity.
- `proxy/parser/tests.rs` H9 confirmed: `test_h9_thread_identity_diverges_after_early_turn_removal` (`assert_ne!()`) documents that archiving early turns changes `fallback_thread_identity` hash (first user+assistant pair is the anchor). After early-turn archival, all subsequent requests get a new session ID, orphaning plans stored under original session. Active bug for all fallback-identity conversations.
- `proxy/rewriter/payload.rs`: `replace_anthropic_content()` always writes system message as `Value::String` regardless of original format — if system was content-block array, format changes to string, invalidating Anthropic's cache prefix for the system block (cache miss until next natural turn resets the prefix).
- `proxy/rewriter/payload.rs`: `replace_message_content()` only replaces the FIRST `text`/`input_text` block in array content — remaining text blocks survive unchanged alongside the stub, leaving stale content in the payload.
- `proxy/rewriter/signals.rs`: `collect_traffic_signals()` parses request body a THIRD time (after parser module parse + rewriter mutation parse) — hot-path waste; fix by accepting pre-parsed `&Value`.
- `proxy/capture/sse.rs`: **tool/function streaming outputs are dropped in SSE reconstruction** (Anthropic/OpenAI Chat/Responses paths are text-only), so tool-use streaming turns can be lost from captured history.
- `proxy/capture.rs`: `finalize_streaming()` marks parse failures as `Complete` instead of `Failed`, hiding reconstruction failures.
- `proxy/handler.rs`: request tracing uses two different UUIDs (span `request_id` vs runtime `request_id`), making correlation between logs and capture records unreliable.
- `proxy/capture/sse.rs`: **[NEW BUG]** `extract_anthropic_final_response` (and OpenAI equivalents) only accumulate `delta.text` — tool_use streaming (`input_json_delta`) is silently dropped. All 3 SSE extractors produce incomplete captures: no ToolUse response blocks for any streaming tool call. Affects dependency tracking, usage heat, staleness for all streaming tool-using turns.
- `proxy/handler.rs`: Duplicate `request_id` generation — `#[instrument]` macro creates one UUID for span, line 38 creates a second. Span IDs and CaptureStore IDs are uncorrelated (can't join logs to captures).
- `proxy/capture.rs`: `evict_if_needed()` iterates DashMap in shard-hash order (not insertion order) → not FIFO; arbitrary entries evicted rather than oldest.
- `proxy/handler.rs` + `proxy/handler/exchange.rs` + `proxy/rewriter.rs`: **R9-DIAG `warn!()` production blast radius is larger than previously known** — 5 total DIAG warns fire on EVERY request: 2 in `rewriter.rs`, 1 in `handler.rs` (SSE stream complete), 2 in `exchange.rs` (ingest start + complete). All 5 need downgrade to `debug!()` in the same cleanup pass.
- `metacog/preview.rs`: **`extract_head` UTF-8 truncation panic** — `head.truncate(MAX_PREVIEW_CHARS)` at byte offset 300, panics on multi-byte char boundary. Same class as `manifest.rs`.
- `metacog/tools.rs`: **`context_search` scope: "all" is a no-op** — tool schema documents this as "includes archived blocks" but function searches same slice regardless of scope parameter. Misleading for the model.
- `metacog/tools.rs`: **`extract_search_snippet` UTF-8 panic** — `&snippet[..max_len]` slices at byte offset after `replace('\n', " ")`. Panics on multi-byte content in snippet window. Same class as preview.rs/manifest.rs.
- `metacog/codex_proxy.rs`: Responses `extract_context_calls` accepts missing `call_id` (`unwrap_or("")`) and emits calls/results with empty IDs, making tool-call/result pairing ambiguous on malformed upstream payloads.
- `mcp/server.rs`: `send_response()` / `send_error()` use `expect()` on stdout writes/flush; broken stdio pipe can panic and kill MCP server instead of graceful shutdown.
- `terminal/codex_bridge.rs`: **no subprocess timeout** — `fetch_thread_blocks()` calls `child.wait_with_output()` with no timeout; if `codex app-server` hangs, the entire bridge thread blocks and `stop_rx` is never checked.
- `terminal/codex_bridge.rs`: **unstable block IDs per poll** — `make_block()` calls `Uuid::new_v4()` on every invocation; each poll cycle generates entirely new block IDs even for unchanged content, breaking engine dedup, fingerprinting, and block reference continuity.
- `terminal/codex_bridge.rs`: startup cursor initializes to EOF (`file_len_or_zero`), so existing active sessions in history are ignored until a new history line is appended.
- `terminal/codex_bridge.rs`: request IDs are second-granularity timestamps (`codex-session-{id}-{unix_secs}`), allowing collisions for multiple emits in the same second.
- `lib.rs`: Unix proxy detach path ignores `setsid()` return value in `pre_exec`; detach failure is silent.

**Smells found:**
- `fn iso_now()` wrapper duplicated in `session.rs`, `action_log.rs`, `versioning.rs` — call `crate::util::iso_now()` directly
- `recount_block_tokens()` in `tokens.rs` is a pointless one-line wrapper around `count_tokens()`
- `tokens_by_zone()` returns `HashMap<String, u32>` but `tokens_by_role()` returns `HashMap<Role, u32>` — inconsistent types in the same file
- `engine/tests.rs`: one assertion is tautological and three tests use `ContextEngine::new()` (real SQLite path), reducing test signal quality and isolation
- `engine/planner/mod.rs`: `format_tokens()` private fn duplicated in both `mod.rs` and `applicator.rs` — confirmed; extract both to `engine::format` shared module
- `engine/planner/mod.rs`: 8 legacy `LEGACY_SESSION_ID` wrapper methods — only needed until tests migrate to session-aware API, then can be removed
- `engine/planner/mod.rs`: `is_batch_point_for_session()` and `check_alert_level_change_for_session()` have implicit call-order dependency — undocumented
- `proxy/runaway_guard.rs`: proxy hard-limit alerts remain cooldown-gated (unlike context hard-limit behavior), potentially under-signaling sustained proxy burst incidents.
- `events/broadcaster.rs`: bounded broadcast queue can still drop events for lagging receivers; sender-side `send()` error cannot detect overflow (drops are surfaced as receiver lag).
- `metacog/tools.rs`: `format_tokens()` is now the **third copy** (mod.rs + applicator.rs + tools.rs, same exact logic). Extract to `crate::util`.
- `metacog/runtime.rs`: `context_tools_mode()` reads env var on every call (syscall per request); `detect_runtime` has over-broad `/responses` path match that could misclassify non-OpenAI paths.
- `metacog/codex_proxy.rs`: `inject_tools()` calls `context_tool_definitions()` directly instead of `self.tool_definitions()` — bypasses any per-runtime overrides.
- `metacog/tools/tests.rs`: `mock_budget` hardcodes alert thresholds (80/90/95%) duplicating engine constants — same smell as `engine/tests.rs`.
- `metacog/runtime.rs`: `parse_context_tools_mode()` is fail-open on unknown values, so env-var typos silently re-enable tools.
- `metacog/tools/plan.rs`: `control.op` parsing is strict lowercase; semantically-valid mixed-case ops from models are rejected.

---

## Architecture Reference

Ownership map — what each module owns and must not do:

| Module | Owns | Must Not |
|--------|------|----------|
| **Parser** (`proxy/parser/*`) | Wire parsing → canonical `Block` records, thread identity, overhead estimation | Mutate engine state or apply policy |
| **Rewriter** (`proxy/rewriter/*`) | JSON mutation, runtime cleanup, trailing injection | Decide archival/compression policy |
| **Planner** (`engine/planner/*`) | Mutation planning, staged plans, heuristics, persistent archive intent | Patch provider JSON directly |
| **Engine** (`engine/`) | Session/block state, ingest, persistence, policy-enforced mutations | Parse provider wire formats |
| **Handler** (`proxy/handler/*`) | Upstream routing, transport filtering, flow orchestration | Own provider JSON transformation |
| **Interceptor** (`proxy/interceptor/*`) | Context-tool interception, bounded reinvoke | Own session state or planner policy |
| **Capture** (`proxy/capture/*`) | Capture store lifecycle, SSE reconstruction | Own session policy or rewrite decisions |
| **MCP** (`mcp/*`) | JSON-RPC transport, tool routing, session affinity forwarding | Own planner semantics or mutation policy |

### Key Constraints (Don't Violate These While Exploring)
- **Stateless clients**: All major LLM coding tools send full conversation history each request. Aperture re-applies archive mutations every turn to keep forwarded prefix stable.
- **API invariants**: Every `tool_use` needs a `tool_result`; non-empty content blocks required; turn alternation (user/assistant) must be maintained.
- **Cache hierarchy (Anthropic)**: tools → system → messages. Changes at any level invalidate that level + all below.

### P0 Mitigations (Keep These in Mind During Code Review)
- Argument validation, output size caps (8KB normal, 2KB compact)
- Proxy runaway guard (rolling window, fail-open)
- Circuit breaker (60s lockout on 24+ calls/60s)
- Kill switch (`APERTURE_CONTEXT_TOOLS_MODE=passive|disabled|off`)
- Orphan sanitizers (both directions)
- Deterministic block IDs, staged planning controls

---

## Backlog (Post-Refactor)
- Block ID display aliases (B1/B42 style mapped to UUIDs)
- Fix breadcrumb delta bug (shows +0 on re-archival, low severity)
- Fix budget % gap (overhead not included in engine calc)
- Fix D: Cache + Archival Death Spiral (cache-aware archival strategy)
- P1: Economics Ledger (token cost instrumentation)
- P3: Schema Overhead Reduction (consolidate tools, lazy injection)
