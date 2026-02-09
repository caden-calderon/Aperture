# Provider Modularity Plan - 2026-02-09

## Objective
Prepare a focused Phase 2 refactor/review pass that keeps the provider surface modular while preserving a single provider-neutral core for context/state handling.

## Scope for Next Session
1. Review current provider launch/integration code for boundary leaks.
2. Normalize provider lifecycle state transitions and event emission contracts.
3. Prepare adapter points for additional providers:
   - Gemini CLI
   - OpenCode
   - KiloCode
4. Keep proxy/parser/provider-specific behavior at boundaries only.

## Architectural Direction

### Core Rule
Provider-specific behavior belongs only in:
- launch adapters (CLI/env/bootstrap)
- transport adapters (proxy vs direct terminal bridge)
- parser adapters (provider event/text normalization)

Provider-neutral behavior belongs in:
- context block store
- zone/session management
- token budget/status display models
- UI store/event reducers

### Adapter Contract (Target)
Each provider adapter should expose a common contract:
1. `id` and display metadata
2. launch strategy (`button quick-launch` + optional command autodetect)
3. connect/disconnect lifecycle hooks
4. stream parser into unified block events
5. capability flags (supports thinking visibility, usage metrics, resume id extraction)
6. health/status mapping into unified status bar states

### Transport Modes
1. **Proxy Mode**: API-level forwarding/parsing (Anthropic/OpenAI parity paths).
2. **Direct CLI Bridge Mode**: terminal output parsing + event bridge (current Codex direct path, Claude direct-compatible path).

## Future Provider Notes

### Providers to Add
1. Gemini CLI
2. OpenCode
3. KiloCode

### What to Reuse
1. Existing quick-launch selector pattern.
2. Existing status/event bridge contract.
3. Existing context block normalization flow.

### What to Isolate Early
1. Provider-specific output parsing regex/patterns.
2. Provider-specific auth/bootstrap assumptions.
3. Provider-specific usage/reasoning visibility behavior.

## Risk Assessment
Low-to-moderate risk if adapter boundaries remain strict.

Main failure modes:
1. Provider output parser behavior leaking into global stores.
2. Status transitions being inferred in multiple places instead of one lifecycle reducer.
3. Hardcoded launch/env behavior that assumes OpenAI/Anthropic semantics.

## Success Criteria for Refactor Prep
1. One provider contract/interface that all providers implement.
2. No provider conditionals in core context engine/store logic.
3. Clear path to add a new provider by adding one adapter + parser, not editing core.
4. Existing Claude/Codex behavior unchanged from user perspective.
