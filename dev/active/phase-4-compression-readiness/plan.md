# Phase 4 Compression Plan (2026-02-13)

## Objective
Implement sidekick-driven compression (bulk/autonomous path) while preserving Phase 3 behavior and fail-open proxy guarantees.

## Architecture Decisions

### Decision 1: Additive rollout by checkpoints
- Checkpoint A: contracts and settings plumbing only.
- Checkpoint B: real provider adapters + queue worker execution.
- Checkpoint C: planner integration for autonomous compression and preserve-keys enforcement.
- Checkpoint D: quality scoring, telemetry, and UX polish.

Rationale:
- Keeps regressions low by introducing behavior changes only after contracts/tests are stable.

### Decision 2: Fail-open by default
- Provider/network failures must not block proxy forwarding.
- Sidekick failures should result in no-op compression (original content preserved).

Rationale:
- Protects core tool flow and avoids introducing hard dependencies in request path.

### Decision 3: Shared typed settings contract
- One Rust settings type (`CompressionSettings`) is source-of-truth.
- Frontend reads/writes this contract via Tauri IPC.

Rationale:
- Avoids drift across engine/UI and simplifies future persistence/migrations.

## Checkpoint A (Completed)
- Implemented compression backend/settings types and normalization.
- Added provider trait + fail-open helper behavior.
- Added async queue contract primitives.
- Wired settings into engine state and Tauri commands.
- Added settings controls in UI and store wiring.
- Added Rust + frontend tests and ran full validation.

## Checkpoint B (Next)
- Implement concrete providers (Anthropic/OpenAI/OpenRouter).
- Add queue runner abstraction (background worker + retry/backoff policy).
- Add integration tests for provider failure and queue processing.

## Guardrails
- Do not regress Wave 3 tool interception/rewrite/capture flow.
- Keep architecture boundaries: planner decides, rewriter applies, sidekick remains optional.
- Avoid blocking I/O in proxy request/response path.
