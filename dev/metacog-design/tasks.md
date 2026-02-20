# Phase 3 Remediation Tasks (2026-02-13)

## Wave 1 — Correctness
- [x] Fix planner budget ceiling application in heuristic path.
- [x] Fix re-invoke loop lifecycle ordering so context tool conversation state is preserved correctly.
- [x] Capture intercepted/re-invoked response body (not original upstream body).
- [x] Add tests for context-only re-invoke, mixed tool calls, depth limit, timeout fail-open.

## Wave 2 — Durable State
- [x] Persist archive/compress/update semantics in engine state between turns.
- [x] Align capture/rewrite/ingest ordering with durable mutations.
- [x] Wire planner signals from real proxy traffic (files/task-boundary/file-mutations).
- [x] Add round-trip tests for persistence across multiple turns.

## Wave 3 — Contract/UX/Test Hardening
- [x] Generate MCP tool schema from shared runtime definitions.
- [x] Align frontend threshold math with planner policy.
- [x] Pass budget ceiling through to budget bar usage site(s).
- [x] Replace weak optional assertions in tool lifecycle integration tests.
- [x] Resolve current Svelte warnings in touched components.

## Validation
- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo test`
- [x] `npx vitest run`
- [x] `npm run check`

## Remaining Blockers
- None for Wave 3 remediation.
