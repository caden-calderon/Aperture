# Phase 1.5: Stability & Modularity Hardening

**Status**: COMPLETE
**Goal**: Harden the Phase 1 foundation before Phase 2 so upcoming features do not accumulate avoidable operational/performance debt
**Prerequisites**: Phase 1 complete
**Estimated Scope**: ~15-25k context

---

## Context from Phase 1

Phase 1 delivered:
- Proxy capture/forwarding and streaming bridge
- Anthropic/OpenAI request/response parsing
- Frontend live context updates and terminal launch flows
- Provider quick-launch support and Codex direct bridge path

Follow-up review identified hardening priorities:
1. Reduce avoidable background work in Codex direct bridge path.
2. Keep provider-specific behavior behind adapter boundaries.
3. Keep frontend bundle growth under control before Phase 2+ feature expansion.

---

## Problem Statement

1. **Bridge churn risk**: Codex direct bridge polled aggressively and spawned subprocess work at a fixed cadence.
2. **Expansion pressure**: Provider onboarding needed explicit templates/contracts for future additions.
3. **Bundle risk**: Build warned on a large client chunk; without intervention this tends to worsen with feature growth.

---

## Deliverables

### 1. Provider Modularity Contract
- Single frontend provider adapter contract with:
  - provider id + labels + launch command
  - transport mode
  - startup marker hints
  - capability flags (`supportsUsage`, `supportsReasoning`, `supportsResumeId`)
- Backend parser adapter trait and registry with:
  - Anthropic adapter
  - OpenAI adapter
  - placeholders for Gemini CLI / OpenCode / KiloCode

### 2. Terminal Launch Reliability
- Quick-launch guarded against re-entrant launch races.
- Manual launch detection converges to same lifecycle/state path as quick-launch.
- Atomic event-listener registration for terminal sessions.

### 3. Codex Bridge Performance Hardening
- Adaptive polling interval backoff for unchanged/idle/error cycles.
- Reset to fast polling on session change/new data.
- Poll timing instrumentation at debug level.

### 4. Frontend Build Hardening
- Manual chunk splitting for heavy dependencies (`xterm`, `prism`, `tauri api`).
- Keep current app behavior unchanged while reducing warning pressure and future bundle coupling.

### 5. Documentation & Onboarding
- Provider onboarding checklist template.
- Provider capability matrix doc.
- RESUME + active task checklist sync.

---

## Files Created / Modified

| File | Action | Purpose |
|------|--------|---------|
| `src/lib/utils/providerAdapters.ts` | Modify | Frontend provider contract + capability flags + manual command inference |
| `src/lib/components/features/Terminal.svelte` | Modify | Launch/lifecycle hardening + manual launch convergence |
| `src/lib/components/layout/TerminalPanel.svelte` | Modify | Adapter-driven quick-launch UI + capability visibility |
| `src-tauri/src/proxy/provider_adapter.rs` | **NEW** | Backend parser adapter trait + builtin adapters |
| `src-tauri/src/terminal/codex_bridge.rs` | Modify | Adaptive poll backoff + instrumentation |
| `vite.config.js` | Modify | Manual vendor chunk splitting |
| `dev/active/provider-modularity-2026-02-09/provider-onboarding-template.md` | **NEW** | Provider onboarding workflow |
| `docs/PROVIDER_CAPABILITY_MATRIX.md` | **NEW** | Capability contract reference |

---

## Test Coverage

### Unit / Integration Additions
- `src/lib/utils/providerAdapters.test.ts`
  - adapter ordering and contract checks
  - capability summary formatting
  - manual command inference
- `src-tauri/src/proxy/provider_adapter.rs` tests
  - adapter detection by path
  - placeholder adapter availability
  - capability checks
- `src-tauri/src/terminal/codex_bridge.rs` tests
  - polling backoff behavior

### Validation
- `make check` passes
- `npm run build` passes

---

## Success Criteria

- [x] Provider contracts are explicit and centralized (frontend + backend parser boundary).
- [x] Manual and quick-launch provider behavior converge through same lifecycle states.
- [x] Codex bridge reduces unnecessary poll frequency in unchanged/idle periods.
- [x] No regressions in proxy/event bridge validation.
- [x] Build and tests pass with hardening changes.
- [x] Phase 2 can proceed on a cleaner base.

---

## Next Phase Handoff

Proceed to `.context/phases/phase-2.md` with these constraints:
- Keep provider-specific logic only in adapters/parsers/transport boundaries.
- Keep core engine/store provider-neutral.
- Keep hot paths observable with timing instrumentation and avoid blocking work.
