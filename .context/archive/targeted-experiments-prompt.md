# Aperture Targeted Experiments Prompt (Post-Round-4, Pre-Fix)

Read these first, in order:
1. `.context/RESUME.md`
2. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-4-consolidated-2026-02-19.md`
3. `dev/active/phase-4-compression-readiness/tasks.md` (see "Targeted Experiments" section)

## Mission

Run targeted experiments to validate fix approaches for the two CRITICAL bugs before implementing any production fixes. We are past the forensic diagnostics phase — root causes are proven with quantitative evidence. Now we need to confirm that the specific fix mechanisms will work.

## Context (do not re-investigate these — they are proven)

- **CRITICAL-1**: Partial-turn archives produce zero payload reduction. Applicator requires full-turn coverage. Projection overstates savings.
- **CRITICAL-2**: `is_context_tool_name()` misses `mcp__aperture__*` namespaced names. One-line fix, high confidence.
- **HIGH-1**: Auxiliary session flips active session → false archival toasts in UI.
- **Cascading failure**: C1+C2 together mean archival ADDS ~6-8k tokens per cycle instead of reducing.

## Experiments to Run (in order)

### 1. CRITICAL-2 Fix (just do it — no experiment needed)
- Fix `is_context_tool_name()` in `src-tauri/src/metacog/runtime.rs` to match `mcp__aperture__aperture_context_*` names
- Convert the 3 existing "proving the bug" replay tests into "proving the fix works" tests
- Run full test suite to confirm no regressions
- This is a one-line string matching fix with zero architectural risk

### 2. Anthropic API Content-Block Removal Experiment (for CRITICAL-1)
- Craft minimal API requests (use `curl` or a test harness) to the Anthropic Messages API
- Test: can you send a user message with some tool_result content blocks removed from the `content` array?
- Test edge cases: single remaining block, only tool_results remaining, empty content array
- This determines whether Option A (block-level removal) is viable

### 3. Orphan Sanitizer Interaction Analysis (for CRITICAL-1)
- Trace the code path: if block-level removal creates orphan tool_use blocks, does the existing sanitizer catch them?
- Write a unit test that simulates the interaction
- This determines whether Option A needs additional sanitizer work

### 4. Cache Economics Model (for CRITICAL-1)
- Quick math: one-time cache_create cost of modifying the payload vs per-request savings from smaller payload
- Determine break-even point in number of requests
- This informs whether the cache penalty is acceptable

### 5. Session Flip Frequency Check (for HIGH-1)
- Determine if Haiku classifier traffic actually routes through the Aperture proxy in normal usage
- If yes: measure how often it flips the active session
- If no: downgrade HIGH-1 severity

## After Experiments

Once experiments are done, implement fixes in order:
1. CRITICAL-2 (cleanup naming — already done in experiment 1)
2. CRITICAL-1 (partial-turn archival — approach determined by experiments 2-4)
3. HIGH-1 (session flips — severity determined by experiment 5)

Each fix must have a failing test first, then the fix, then full test suite validation.

## Hard Constraints
- Do not re-investigate proven root causes (we have 4 rounds of evidence)
- Do not implement CRITICAL-1 fix until experiment 2 confirms the API behavior
- Run `cargo test` + `cargo clippy -- -D warnings` after every change
- Update `dev/active/phase-4-compression-readiness/tasks.md` as experiments complete
