# Round 6 Deep-Dive Diagnostics

Read and execute exactly: this prompt. Do NOT write any code or fixes yet.

## Mission

Trace 3 new bugs found in the Round 5 manual test to their exact root cause in the rewriter/cleanup pipeline. We need to be **certain** of each bug's cause before writing a single line of fix code.

## Context

Read these files first, in order:
1. `.context/RESUME.md` — current state, what's fixed, what's broken
2. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-5-2026-02-19.md` — full timeline with line numbers, evidence, and hypotheses
3. `dev/active/phase-4-compression-readiness/tasks.md` — investigation subtasks per bug

The manual test log is at: `~/.claude/projects/-home-caden-projects-Aperture/db654aac-a155-4291-aae6-2cb1dfd20b31.jsonl`

## Bugs to Investigate (priority order)

### BUG #1 (P0): "This model does not support assistant message prefill"
4 occurrences. After Aperture's cleanup strips MCP tool calls from history, the payload sometimes ends with an assistant message instead of a user message. Opus 4.6 rejects this.

**Investigation steps:**
1. Read the rewrite orchestration flow: `src-tauri/src/proxy/rewriter.rs` — find the order of operations (cleanup → sanitize → payload apply → forward)
2. Read `src-tauri/src/proxy/rewriter/sanitize.rs` — find `sanitize_anthropic_orphan_tool_uses()` and `sanitize_anthropic_orphan_tool_results()`
3. Read `src-tauri/src/engine/planner/cleanup.rs` — find `cleanup_history()` and how it strips MCP tool calls
4. Answer: does cleanup run BEFORE or AFTER the orphan sanitizers? If cleanup creates orphans that the sanitizer never sees, that's the bug.
5. Answer: does the sanitizer check the LAST message specifically, or only scan for orphans mid-conversation?
6. Check the JSONL at L79 — what was the last message role in the request that got the 400?

### BUG #3 (P1): "thinking or redacted_thinking blocks cannot be modified"
1 occurrence at L217, after L196 produced a thinking block. Anthropic requires thinking blocks to be passed through verbatim.

**Investigation steps:**
1. Read `src-tauri/src/proxy/rewriter/payload.rs` — does `apply_stubs_to_anthropic()` iterate over ALL content blocks including thinking?
2. Read `src-tauri/src/engine/ingest.rs` — does ANSI stripping or internal prompt filtering modify thinking block content?
3. Read `src-tauri/src/proxy/rewriter/sanitize.rs` — do any sanitizers touch thinking blocks?
4. Grep for `thinking` across the rewriter and engine to find every place thinking content could be modified
5. Answer: is there ANY guard that skips `thinking`/`redacted_thinking` type blocks?

### BUG #2 (P1): "final assistant content cannot end with trailing whitespace"
1 occurrence at L115. The rewriter produced assistant content with trailing whitespace.

**Investigation steps:**
1. Check if stub content or replacement text includes trailing `\n` or spaces
2. Check if content block assembly in the rewriter concatenates with whitespace
3. Check if this is related to BUG #1 — maybe the orphaned assistant message has whitespace-only content after its tool_use blocks are stripped

### BUG #5 (P1): Cache catastrophe — non-deterministic system block IDs
System block IDs changed on every request: `e4916327 → a5417238 → 992504a2 → a13b421c`. This causes cache prefix shifts.

**Investigation steps:**
1. Read `src-tauri/src/proxy/parser/mod.rs` — how are block IDs generated? Content-addressed hash?
2. What content feeds into the system block hash? If timestamps or billing headers are included, that explains the churn.
3. This is lower priority — investigate after the P0/P1 bugs.

## Output

For each bug, write up:
- **Confirmed root cause** (with file:line references)
- **Code path trace** (function A calls B calls C, at step N the invariant breaks)
- **Minimal reproduction scenario** (what input → what broken output)
- **Proposed fix** (description only, no code yet)

Write findings to `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-6-2026-02-19.md`.

Do NOT implement any fixes. Research only. We will review findings together before touching code.
