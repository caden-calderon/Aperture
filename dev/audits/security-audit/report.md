# Aperture Audit Report (Calibrated)
Date: 2026-02-08  
Scope: Full codebase audit per `.context/AUDIT_PROMPT.md`, then calibrated against `review-of-audit.md`  
Positioning: Evidence-based findings, security-forward for future phases

## Validation Commands Run
- `make check` (fails: 8 lint errors)
- `make test` (passes: Rust tests 3/3, frontend test suite empty)
- `npm run check` (passes: svelte-check 0 errors/warnings)
- `npm run build` (passes; desktop-relevant size warnings only)
- `cargo test --manifest-path src-tauri/Cargo.toml` (passes)

---

## 1) Code Quality & Correctness
**Score: 7/10**

### Findings
- Lint gate is red with 8 blocking errors:
  - `src/lib/components/blocks/ContextBlock.svelte:239`
  - `src/lib/components/blocks/ContextBlock.svelte:245`
  - `src/lib/components/layout/Modal.svelte:417`
  - `src/lib/components/layout/Modal.svelte:443`
  - `src/lib/components/features/ContextDiff.svelte:98`
  - `src/lib/components/layout/TitleBar.svelte:41`
  - `src/lib/stores/selection.svelte.ts:17`
  - `src/lib/stores/ui.svelte.ts:30`
- Custom block type path uses an unsafe role cast and inconsistent semantics:
  - cast at `src/lib/components/controls/BlockTypeManager.svelte:88`
  - batch role setter ignores `blockType` at `src/lib/stores/context.svelte.ts:287`
  - selection by type assumes role-only at `src/lib/stores/selection.svelte.ts:92`
- Rust panic surfaces still exist in real backend paths:
  - `src-tauri/src/proxy/mod.rs:57`
  - `src-tauri/src/proxy/mod.rs:69`
  - `src-tauri/src/terminal/session.rs:32`

### Recommendations (priority order)
1. Make `make check` green immediately.
2. Remove custom type role-cast hack; keep `role` canonical and use `blockType` for custom taxonomy.
3. Replace runtime `unwrap/expect` in backend internals with recoverable error handling.

---

## 2) Architecture & Organization
**Score: 7/10**

### Findings
- Good separation of concerns:
  - store barrel pattern at `src/lib/stores/index.ts:5`
  - composable split at `src/lib/composables/index.ts:1`
  - component grouping at `src/lib/components/index.ts:5`
- High-LOC components remain concentration points:
  - `src/lib/components/layout/Modal.svelte` (~1468 LOC)
  - `src/lib/components/features/ContextDiff.svelte` (~934 LOC)
  - `src/lib/components/blocks/Zone.svelte` (~735 LOC)
  - `src/lib/components/layout/ZoneManager.svelte` (~719 LOC)
- Engine/events scaffolding is intentionally thin but phase-appropriate:
  - `src-tauri/src/engine/mod.rs:7`
  - `src-tauri/src/events/mod.rs:7`

### Recommendations (priority order)
1. Split modal/diff into subcomponents before Phase 1 complexity lands.
2. Keep current store/composable boundaries; this is the right structural direction.

---

## 3) Performance
**Score: 7/10**

### Findings
- Heavy stores are debounced correctly:
  - `src/lib/stores/context.svelte.ts:40`
  - `src/lib/stores/zones.svelte.ts:64`
  - `src/lib/stores/editHistory.svelte.ts:30`
- Low-frequency stores still write synchronously to localStorage:
  - `src/lib/stores/ui.svelte.ts:163`
  - `src/lib/stores/theme.svelte.ts:480`
  - `src/lib/stores/blockTypes.svelte.ts:86`
  - `src/lib/stores/terminal.svelte.ts:133`
- Resize/drag paths are optimized with rAF + direct DOM updates:
  - `src/lib/composables/resizable.svelte.ts:61`
  - `src/lib/composables/resizable.svelte.ts:170`

### Recommendations (priority order)
1. Standardize persistence policy (either document immediate writes, or debounce them uniformly).
2. Keep existing rAF resize architecture unchanged.

---

## 4) Documentation Accuracy
**Score: 5/10**

### Findings
- `docs/ARCHITECTURE.md` lists several non-existent files:
  - `docs/ARCHITECTURE.md:66`
  - `docs/ARCHITECTURE.md:67`
  - `docs/ARCHITECTURE.md:68`
  - `docs/ARCHITECTURE.md:69`
- Inventory/integration persistence keys are stale for theme data:
  - `.context/FRONTEND_INVENTORY.md:168`
  - `.context/FRONTEND_INVENTORY.md:169`
  - `docs/INTEGRATION.md:152`
  - actual key in code: `src/lib/stores/theme.svelte.ts:527`
- README phase status is stale:
  - `README.md:5`

### Recommendations (priority order)
1. Fix architecture module map first (highest drift impact).
2. Update localStorage key tables from code, not memory.
3. Refresh README phase status after each phase-close.

---

## 5) CSS & Styling
**Score: 7/10**

### Findings
- Tokenized theme system is strong and coherent from `src/app.css:13`.
- Correct `:global()` usage inside Svelte component styles, e.g. `src/lib/components/features/Terminal.svelte:158`.
- A few hardcoded color values remain:
  - `src/lib/components/layout/TitleBar.svelte:284`
  - `src/lib/components/blocks/ContextBlock.svelte:150`
- Responsive breakpoint strategy is still minimal.

### Recommendations (priority order)
1. Remove remaining hardcoded color fallbacks where practical.
2. Add explicit responsive breakpoints for constrained layouts.

---

## 6) Security
**Score: 6/10**

### Findings
- No confirmed active exploit path was found in current audited flows.
- Security hardening debt exists and should be handled now (before backend scope grows):
  - `csp` is disabled at `src-tauri/tauri.conf.json:25`
  - `{@html}` sinks exist and depend on safe-invariant behavior (`Prism`/escaped paths):
    - `src/lib/components/blocks/ContextBlock.svelte:239`
    - `src/lib/components/layout/Modal.svelte:417`
  - debug logging includes request/response previews:
    - `src-tauri/src/lib.rs:32`
    - `src-tauri/src/proxy/handler.rs:113`
    - `src-tauri/src/proxy/handler.rs:174`
  - local `.env` currently contains real-looking API key material (`.env:13`) even though ignored by git (`.gitignore:20`)

### Recommendations (priority order)
1. Define and implement a Phase 1 security baseline:
   - explicit CSP policy decision
   - documented/guarded `{@html}` invariants
   - logging redaction policy by environment
2. Add tests around rendering invariants (`highlightCode`/`escapeHtml`) to prevent regressions.
3. Rotate local credentials and avoid long-lived key material in workspace `.env`.

Note: This is a strategic hardening recommendation, not a claim of immediate RCE/XSS compromise.

---

## 7) Accessibility
**Score: 5/10**

### Findings
- `svelte-ignore` accessibility suppressions are still common:
  - `src/routes/+page.svelte:171`
  - `src/routes/+page.svelte:297`
  - `src/lib/components/controls/ContextMenu.svelte:94`
  - `src/lib/components/layout/Modal.svelte:408`
- Dialog semantics exist, but focus trap is missing:
  - `src/lib/components/layout/Modal.svelte:311`
  - `src/lib/components/controls/CommandPalette.svelte:118`
  - `src/lib/components/features/ContextDiff.svelte:279`
- Context menu submenu interactions are hover-centric:
  - `src/lib/components/controls/ContextMenu.svelte:97`
  - `src/lib/components/controls/ContextMenu.svelte:117`

### Recommendations (priority order)
1. Build shared focus-trap utility and apply to all dialogs.
2. Replace suppressed interactions with semantic controls/keyboard support.
3. Add keyboard navigation model for context-menu submenus.

---

## 8) Testing
**Score: 3/10**

### Findings
- Frontend test coverage is effectively zero:
  - vitest reports no test files
  - `vite.config.js:15` (`passWithNoTests: true`) allows silent pass
- Rust tests are narrow (3 tests in proxy handler):
  - `src-tauri/src/proxy/handler.rs:213`
- Manual test script exists but is not in CI:
  - `tests/test_proxy.sh:1`

### Recommendations (priority order)
1. Add baseline frontend unit tests (stores + critical UI flows).
2. Add backend integration tests for proxy and terminal lifecycle.
3. Remove `passWithNoTests` after baseline is in place.

### Top 5 Tests To Add First
1. Custom block type assignment/selection semantics.
2. Snapshot save/switch/restore correctness.
3. Proxy upstream detection + timeout/error mapping.
4. Modal diff/revert state handling.
5. Terminal spawn/cleanup/reconnect event lifecycle.

---

## 9) Build System & Dependencies
**Score: 6/10**

### Findings
- `make check` fails, so the gate is currently broken for phase completion.
- `make check` excludes `npm run build` and `cargo fmt --check`:
  - `Makefile:38`
- Warning tolerance currently masks missing frontend tests:
  - `vite.config.js:15`
- Lockfiles are correctly tracked:
  - `package-lock.json:1`
  - `src-tauri/Cargo.lock:1`

### Recommendations (priority order)
1. Restore check gate first (lint pass).
2. Expand `make check` to include build + fmt checks.
3. Audit actual dependency usage before adding more packages.

---

## 10) Readiness for Phase 1
**Score: 7/10**

### Findings
- Proxy forwarding foundation is good:
  - `src-tauri/src/proxy/mod.rs:81`
  - `src-tauri/src/proxy/handler.rs:33`
- Engine/events scaffolding exists and is extendable:
  - `src-tauri/src/engine/mod.rs:7`
  - `src-tauri/src/events/mod.rs:7`
- Main readiness drag is quality process debt (lint/tests/docs), not architecture failure.
- Type contract between TS and Rust is not yet exercised on-wire; plan it before live sync, but it is not a current production defect.

### Recommendations (priority order)
1. Enter Phase 1 only after `make check` is green and baseline tests exist.
2. Define TS↔Rust serialization contract when wiring real IPC/events.
3. Apply security baseline now to avoid future migration churn.

---

## Weighted Overall Score
**Overall: 6.3/10**

Weighting emphasis:
- Testing and phase-readiness weighted highest.
- Security weighted high with future-phase lens (hardening-now strategy).

---

## Top 10 Issues (Ranked)
1. `make check` failing (8 blocking lint errors).
2. Frontend test suite absent.
3. Documentation drift in architecture/integration/inventory.
4. Custom block-type role-cast/type semantics debt.
5. Runtime `unwrap/expect` in backend internals.
6. Security hardening baseline not yet formalized (CSP/html invariant/logging policy).
7. Accessibility: no focus trap for dialogs.
8. Immediate localStorage writes are inconsistently handled.
9. Large components raise maintenance risk.
10. README phase status is stale.

---

## Top 5 Strengths
1. Good store/composable/component separation.
2. Strong resize-path implementation (rAF + commit-on-end).
3. Debounced persistence in high-churn stores.
4. Terminal lifecycle cleanup/listener teardown is implemented.
5. Proxy core is functional and tested.

---

## Phase 1 Blockers (Must Fix Before Starting)
1. Green quality gate (`make check`).
2. Baseline automated frontend tests.
3. Fix custom block-type semantics and backend panic surfaces.
4. Security baseline decisions documented and implemented (future-scale hardening).

---

## Quick Wins (<30 Minutes, High Impact)
1. Fix `prefer-const`, remove dead function, remove stale svelte-ignore.
2. Add explicit safe-usage lint suppressions for reviewed `{@html}` sinks.
3. Patch architecture/inventory/integration doc drift.
4. Add clipboard error handling on write operations.
5. Update README status line to current phase.

---

## Remediation Plan

### P0 (Now, before Phase 1)
1. Restore green `make check`.
2. Fix block-type semantics (`role` vs `blockType`) and remove unsafe casts.
3. Replace backend `unwrap/expect` in proxy/terminal internals.
4. Apply security baseline:
   - decide CSP path
   - codify + test `{@html}` invariants
   - set logging policy for debug vs normal runs

### P1 (Early Phase 1)
1. Add frontend test baseline and remove `passWithNoTests`.
2. Add integration tests for proxy + terminal behavior.
3. Add focus-trap utility and apply to all dialog components.

### P2 (Phase 1 In-Flight)
1. Split modal/diff into child components.
2. Standardize persistence write policy across stores.
3. Keep docs synchronized as part of phase-close checklist.
