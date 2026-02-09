# Provider Modularity Tasks - 2026-02-09

## Next Session Checklist

## Architecture
- [x] Define or confirm a single provider adapter contract (launch, lifecycle, parse, capabilities, health mapping).
- [x] Verify core engine/store remains provider-neutral.
- [x] Move any leaked provider conditionals to adapter/parser boundaries.

## Claude + Codex Parity
- [x] Verify manual terminal launch and button launch follow the same bridge path.
- [x] Verify disconnect/provider-switch clears context and status consistently.
- [x] Verify launch status states are deterministic and sourced from one reducer.

## Expansion Readiness
- [ ] Add provider onboarding template/checklist for Gemini CLI, OpenCode, KiloCode.
- [x] Ensure adding a provider requires no core store edits.
- [ ] Add adapter capability matrix doc (available metadata, reasoning visibility, usage counters).

## Performance + Reliability
- [x] Re-check proxy and stream hot paths for blocking calls.
- [x] Keep parsing/enrichment off forwarding critical path when possible.
- [x] Confirm instrumentation exists for request/stream timing regressions.

## Validation
- [x] Run `make check`
- [x] Run `npm run build`
- [x] Add/adjust tests for provider lifecycle transitions and parser normalization edge cases.

## Follow-up Tasks
- [ ] Add backend parser adapter trait to mirror frontend provider adapter contract for Gemini CLI/OpenCode/KiloCode.
- [ ] Add explicit terminal/provider lifecycle tests around manual `claude` command detection and status convergence.
- [ ] Add capability flags (`supports_usage`, `supports_reasoning`, `supports_resume_id`) to the provider adapter contract and expose in UI.
