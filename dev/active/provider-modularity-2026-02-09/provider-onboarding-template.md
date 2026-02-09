# Provider Onboarding Template

Use this checklist when adding a new provider (Gemini CLI, OpenCode, KiloCode, etc.).

## 1. Adapter Registration
- [ ] Add frontend adapter entry in `src/lib/utils/providerAdapters.ts`:
  - [ ] `id`, labels, command, transport mode
  - [ ] capability flags (`supportsUsage`, `supportsReasoning`, `supportsResumeId`)
  - [ ] startup markers (if direct CLI bridge mode)
- [ ] Add backend parser adapter metadata in `src-tauri/src/proxy/provider_adapter.rs`.

## 2. Launch + Lifecycle
- [ ] Confirm quick-launch button path works from `TerminalPanel`.
- [ ] Confirm manual terminal command path converges via command inference/output markers.
- [ ] Confirm launch states are deterministic: `idle -> launching -> running|error -> idle`.
- [ ] Confirm session reset occurs on provider switch.

## 3. Parsing Boundary
- [ ] Keep provider-specific parsing in proxy/parser adapter boundaries only.
- [ ] Do not add provider conditionals to context store or core engine modules.
- [ ] Add request/response parse tests for provider-native payload shapes.

## 4. Event Bridge Contract
- [ ] Emit `request_captured`, `blocks_captured`, `response_complete` parity events.
- [ ] Ensure stream progress stays on `aperture:stream-progress`.
- [ ] Verify block conversion and zone defaults stay provider-neutral.

## 5. Performance + Reliability
- [ ] Avoid blocking operations on proxy forwarding path.
- [ ] Keep heavy enrichment off request/stream critical path.
- [ ] Add/keep timing instrumentation for overhead regressions.

## 6. Validation
- [ ] `make check`
- [ ] `npm run build`
- [ ] Manual smoke:
  - [ ] quick-launch
  - [ ] manual command launch
  - [ ] session switch/reset
  - [ ] streaming + block capture
