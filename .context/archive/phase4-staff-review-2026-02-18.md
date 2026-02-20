# Phase 4 Staff Review Report
Date: 2026-02-18
Reviewer: Codex (staff-level pass)
Scope: Docs + dirty working tree + tests/lint

## Executive Summary
- Overall direction is strong: cache-safe trailing injection, staged planning workflow, sanitizer hardening, and budget-overhead accounting are all meaningful improvements.
- Primary blocker remains architectural: planner mutable state is global and can bleed across sessions/threads.
- Secondary blocker: suggestion quality/semantics are too loose for current warning language and can recommend risky blocks.
- Test posture is strong (`cargo test`, `vitest`, `svelte-check` all green), but `clippy -D warnings` is currently broken.

## What Was Reviewed
- Context docs:
  - `.context/RESUME.md`
  - `.context/archive/sorted-tumbling-shamir.md`
  - `.context/archive/velvet-tinkering-garden.md`
  - `.context/archive/zazzy-herding-wave.md`
- Active phase docs:
  - `dev/active/phase-4-compression-readiness/context.md`
  - `dev/active/phase-4-compression-readiness/plan.md`
  - `dev/active/phase-4-compression-readiness/tasks.md`
  - `dev/active/phase-4-compression-readiness/cache-invalidation-analysis.md`
  - `dev/active/phase-4-compression-readiness/manual-test-prompts.md`
  - `dev/active/phase-4-compression-readiness/session-analysis-2026-02-18.md`
- Dirty code changes across engine/planner/proxy/metacog/frontend.

## Validation Results
- `cargo test --manifest-path src-tauri/Cargo.toml`: pass (571 total tests).
- `npx vitest run`: pass (52/52).
- `npm run check`: pass (0 errors, 0 warnings).
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`: fail on `too_many_arguments` at `src-tauri/src/engine/mod.rs:128`.

## Findings (Severity Ordered)

### High
1. Planner state bleed across sessions/threads.
   - Evidence:
     - Global mutable planner fields: `src-tauri/src/engine/planner/mod.rs:36`, `src-tauri/src/engine/planner/mod.rs:38`, `src-tauri/src/engine/planner/mod.rs:47`.
     - Consumed globally during rewrite: `src-tauri/src/proxy/rewriter.rs:83`.
   - Impact:
     - Staged/pending plan and alert-level transitions can leak between unrelated sessions.
   - Required fix:
     - Make planner mutable state session-scoped (pending/staged/last_alert/previous_turn_files/last_delta keyed by session identity).

2. Projected block count can underflow with duplicate archive actions.
   - Evidence:
     - `src-tauri/src/engine/planner/mod.rs:562`, `src-tauri/src/engine/planner/mod.rs:630`.
   - Impact:
     - Invalid projections, potential debug panic.
   - Required fix:
     - Dedup mutation targets per slot before projection, or use saturating arithmetic for projected counts.

3. Suggestion semantics are too permissive vs message wording.
   - Evidence:
     - Non-stale allowed under pressure: `src-tauri/src/engine/planner/heuristics.rs:74`.
     - Recency not excluded in candidate check: `src-tauri/src/engine/planner/heuristics.rs:343`.
   - Impact:
     - Warnings claim "stale blocks suggested" but may include active-context candidates.
   - Required fix:
     - Align candidate policy and warning text.

### Medium
4. `context_preview` suggestions ignore traffic/file signals.
   - Evidence:
     - Synthetic empty signals in `src-tauri/src/metacog/tools.rs:287`.
   - Impact:
     - Lower relevance quality; easier to suggest wrong blocks.

5. Trailing warning injection does not cover all OpenAI content shapes.
   - Evidence:
     - Chat path only appends for string content: `src-tauri/src/proxy/rewriter.rs:792`.
     - Responses path requires `input[]`: `src-tauri/src/proxy/rewriter.rs:800`.
   - Impact:
     - Silent misses on valid payload variants; inconsistent warning behavior.

6. Clippy quality gate currently broken.
   - Evidence:
     - `src-tauri/src/engine/mod.rs:128`.
   - Impact:
     - CI/quality bar regression.

### Low
7. Unreachable compact-limit branch in context API tool dispatch.
   - Evidence:
     - Hard-limit returns before limit selection: `src-tauri/src/proxy/context_api.rs:158`, `src-tauri/src/proxy/context_api.rs:178`.
   - Impact:
     - Dead path / maintenance noise.

8. Commit risk: untracked module file.
   - Evidence:
     - `src-tauri/src/proxy/runaway_guard.rs` untracked while imported by `src-tauri/src/proxy/mod.rs:17`.
   - Impact:
     - Easy-to-miss build break if omitted from commit.

9. Doc conflict on Issue C root cause.
   - Evidence:
     - `.context/archive/sorted-tumbling-shamir.md:12` vs `dev/active/phase-4-compression-readiness/tasks.md:63`.
   - Impact:
     - Operator confusion.

## Decisions for Open Questions

### Q1: Should multi-session isolation be strict?
Yes. This must be treated as a hard requirement.

Recommended policy:
- No planner mutable state shared across session identities.
- Every staged/pending plan, threshold state, and turn-file memory is keyed by session.

### Q2: Should suggestions include recency?
Recommended: two-tier suggestions.

Tier A (default, shown prominently):
- Stale + middle-zone only.
- Used for warning counts and normal recommendations.

Tier B (opportunistic, clearly labeled):
- Recency candidates only when all are true:
  - task boundary detected,
  - utilization >= critical threshold,
  - candidate is not pinned,
  - candidate has low relevance boost.
- Never merge Tier B into "stale blocks suggested" count.
- Present as optional with explicit rationale: "likely previous-task context."

Why:
- Keeps safety high by default.
- Still supports your "new task, old recency stack" scenario.
- Lets LLM reason over edge cases without over-aggressive automatic guidance.

### Q3: Should OpenAI be treated differently from Claude Code?
Recommended: same correctness contract, runtime-specific optimization.

Common contract:
- Never mutate system text for dynamic status.
- Warning/breadcrumb behavior should be semantically consistent.
- Sanitization and fail-open behavior should match.

Runtime-specific:
- Cache economics and tool transport differ (MCP vs injected tools), so optimization strategy can differ.
- But missing warning injection because of payload-shape gaps should still be fixed for OpenAI paths.

## Remediation Plan (Execution Order)
1. Session-scope planner state.
   - Introduce per-session planner state map in engine layer.
   - Refactor planner APIs to accept session key/state handle.
   - Update rewriter to read/write planner state via current active session identity.
2. Fix projection robustness.
   - Dedup mutation targets by slot before projection.
   - Saturating math for projected counts.
   - Add regression tests for duplicate IDs.
3. Tighten suggestion policy.
   - Enforce Tier A/Tier B model.
   - Update warning text/count logic to match real filters.
4. Improve preview suggestion signals.
   - Feed planner real or persisted signal context into preview path (at minimum current-turn files from planner session state).
5. OpenAI trailing injection parity.
   - Handle array-content chat messages and additional responses shapes.
   - Add format-coverage tests.
6. Restore clippy gate.
   - Refactor `ingest` params into a struct, or explicitly justify/allow the lint with rationale.
7. Cleanup/doc consistency.
   - Track/add `runaway_guard.rs` before commit.
   - Resolve Issue C narrative mismatch in docs.

## Exit Criteria Before Re-Running Manual Phase 4 Test
- No cross-session planner state leakage in integration tests.
- Suggestion output matches stated policy and warning language.
- OpenAI and Anthropic warning injection behavior validated by tests.
- `cargo test`, `vitest`, `npm run check`, and `clippy -D warnings` all green.

## Residual Risk If You Proceed Without Fixes
- Cross-session staged plan contamination (highest risk).
- LLM acting on low-quality suggestions and archiving relevant context.
- Inconsistent warning behavior by provider/payload shape.
