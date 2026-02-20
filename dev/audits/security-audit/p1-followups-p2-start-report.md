# P1 Follow-ups Complete + P2 Started
Date: 2026-02-08

## Scope Executed This Session

Completed the outstanding P1 follow-up recommendations from `p1-execution-report.md` and began P2 refactor work.

## P1 Follow-ups Completed

### 1) Rust failure-path tests added

- `src-tauri/src/proxy/mod.rs`
  - Refactored client construction through `ProxyState::build_client(...)` to enable direct error-path testing.
  - Added tests:
    - `test_proxy_state_with_config_uses_custom_urls`
    - `test_build_client_maps_reqwest_builder_errors` (invalid user-agent -> `ClientBuildFailed`)

- `src-tauri/src/terminal/session.rs`
  - Extracted reader initialization into `clone_session_reader(...)`.
  - Added tests:
    - `test_clone_session_reader_maps_clone_failure`
    - `test_clone_session_reader_maps_poisoned_lock`

Result: backend panic-reduction paths now have explicit unit coverage.

### 2) Focus trap utility extended for nested popovers

- `src/lib/utils/focusTrap.ts`
  - Added `additionalRoots` option to include focusable elements rendered outside the main dialog root.
  - Updated trap listener to document-capture mode so Tab/Escape handling works across additional roots.
  - Preserved previous-focus restore behavior.

- `src/lib/components/layout/Modal.svelte`
  - Wired `additionalRoots` with role and zone dropdown roots.

### 3) CI/frontend test existence assertion

- `Makefile`
  - Added `assert-frontend-tests` target:
    - fails when no `.test` / `.spec` TS/JS files exist in `src/` or `tests/`
  - Updated `test-ui` to depend on this assertion.

- `.github/workflows/ci.yml`
  - Added frontend CI workflow:
    - install deps
    - assert frontend tests exist
    - run lint/check/test/build

## P2 Started

### Component decomposition kickoff

- Added `src/lib/components/features/ContextDiffStats.svelte`
  - Extracted summary stats UI from `ContextDiff.svelte`.
- Updated `src/lib/components/features/ContextDiff.svelte`
  - Replaced inline stats markup with `<ContextDiffStats .../>`.
  - Removed now-local stats CSS from parent component.
- Updated `src/lib/components/index.ts` export list.

### Continued decomposition (this session)

- Added `src/lib/components/features/ContextDiffEntry.svelte`
  - Extracted each diff row + inline line-diff + revert actions from `ContextDiff.svelte`.
- Updated `src/lib/components/features/ContextDiff.svelte`
  - Delegates entry rendering/state callbacks to `<ContextDiffEntry .../>`.
  - Removed entry-level style block from parent.
- Added `src/lib/components/layout/ModalTypeDropdown.svelte`
  - Extracted modal block-type selector/dropdown from `Modal.svelte`.
- Updated `src/lib/components/layout/Modal.svelte`
  - Uses `<ModalTypeDropdown .../>` and removed duplicated dropdown style block.
- Updated `src/lib/components/index.ts` exports for new components.

Current impact:
- `ContextDiff.svelte`: `928` LOC -> `576` LOC
- `Modal.svelte`: `1474` LOC -> `1387` LOC

### Continued decomposition (latest pass)

- Added `src/lib/components/layout/ModalZoneDropdown.svelte`
  - Extracted modal zone selector/dropdown from `Modal.svelte`.
- Updated `src/lib/components/layout/Modal.svelte`
  - Replaced inline zone dropdown block with `<ModalZoneDropdown .../>`.
  - Removed now-local zone dropdown styles and simplified focus-trap wiring.
- Added `src/lib/components/features/ContextDiffSelector.svelte`
  - Extracted snapshot selector panel (simple mode + from/to mode toggle and selectors).
- Updated `src/lib/components/features/ContextDiff.svelte`
  - Replaced inline selector panel with `<ContextDiffSelector .../>`.
  - Removed selector-local style block.
- Updated `src/lib/components/index.ts` exports for both new components.

Updated impact:
- `ContextDiff.svelte`: `928` LOC -> `459` LOC
- `Modal.svelte`: `1474` LOC -> `1290` LOC

### Continued decomposition (final pass)

- Added `src/lib/components/layout/ModalActionPanel.svelte`
  - Extracted modal action section: zone/compression/pin/snapshot controls and remove action.
- Updated `src/lib/components/layout/Modal.svelte`
  - Replaced inline action block with `<ModalActionPanel .../>`.
  - Removed action-panel-local CSS and state plumbing now owned by the extracted component.
- Updated `src/lib/components/index.ts` exports for `ModalActionPanel`.

Final impact:
- `ContextDiff.svelte`: `928` LOC -> `459` LOC
- `Modal.svelte`: `1474` LOC -> `1094` LOC

## Verification

### `make check`
- PASS
- Rust tests: 7 passed
- Vitest: 2 files, 8 tests passed

### `make test`
- PASS
- Rust tests: 7 passed
- Vitest: 2 files, 8 tests passed

### `npm run build`
- PASS
- Existing non-blocking chunk-size warning remains

## Next P2 candidates

1. Extract diff entry row + inline diff panel from `ContextDiff.svelte`.
2. Extract modal type/zone dropdown controls from `Modal.svelte`.
3. Isolate snapshot/version timeline logic into dedicated modal subcomponent/composable.
