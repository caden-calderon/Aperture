# Phase 0.5: Foundation Hardening

## Context

Phase 0 massively overdelivered — 20 components, 10 stores, embedded terminal, snapshot branching, syntax highlighting, and more. Five parallel audits scored the codebase 8.5/10 on structure but found critical gaps in performance (5/10), documentation currency, and backend readiness for Phase 1. This plan hardens all foundations before wiring live proxy data.

**Audit commit `e756158` already completed:**
- Extracted `escapeHtml`, `getPreview`/`truncate` to shared utils (`text.ts`, `syntax.ts`)
- Removed duplicate utility functions from ContextBlock, Modal, ContextDiff
- Converted `selectedIds` (selection store) and `collapsedZones` (UI store) from Set → SvelteSet
- Real TCP health check in `is_proxy_running()`
- Optional cols/rows in `spawn_shell`, dimension validation in `resize_terminal`
- Replaced `.expect` panic with graceful error logging
- Tracked `Cargo.lock`, removed broken Makefile target

**What remains (this plan):**
- `+page.svelte` is still 1,963 LOC total (752 LOC script) — needs composable extraction
- 38 synchronous `saveToLocalStorage()` calls with zero debouncing
- `JSON.parse(JSON.stringify())` deep clones (2 instances) — replace with `structuredClone`
- Components still flat (20 files in one directory)
- No backend module skeletons for engine or events
- Proxy handler not split from `mod.rs`

---

## Session 1: Code Changes

### Subagent A: Extract Composables from +page.svelte

**Goal:** Reduce script section from ~752 LOC to ~250 LOC by extracting handler logic into `src/lib/composables/`.

**New files:**

| File | ~LOC | Extracts |
|------|------|----------|
| `resizable.svelte.ts` | 180 | Sidebar, zone, terminal resize handlers + state |
| `blockHandlers.svelte.ts` | 68 | Block select/doubleclick/drag/contextmenu + context menu state |
| `modalHandlers.svelte.ts` | 46 | Modal compress/move/pin/remove/edit/role handlers |
| `keyboardHandlers.svelte.ts` | 133 | J/K/arrow nav, Ctrl+T, Ctrl+F, A, Esc, Del, etc. + `allBlocksFlat` derived |
| `commandHandlers.svelte.ts` | 182 | Full command palette dispatch (`handleCommand`) |
| `index.ts` | 6 | Barrel re-export |

Each file exports a `create*()` factory function that takes store/ref dependencies and returns state + handlers.

**Resulting +page.svelte script:** ~250 LOC (imports, composable instantiation, remaining state, onMount, $effects, event wiring).

**Critical files:**
- `src/routes/+page.svelte` — extract ~550 LOC
- `src/lib/composables/` — 6 new files

---

### Subagent B: Component Subdirectories

**Goal:** Organize 20 flat components into logical subdirectories.

```
src/lib/components/
├── blocks/          ContextBlock, Zone, Sparkline
├── layout/          Modal, TerminalPanel, TitleBar, ZoneManager
├── controls/        CommandPalette, ContextMenu, SearchBar, ThemeToggle, ThemeCustomizer, BlockTypeManager
├── features/        ContextDiff, ZoneMinimap, Terminal
├── ui/              CanvasOverlay, Toast, TokenBudgetBar, DensityControl
└── index.ts         (updated barrel)
```

**Import updates required:**
1. `index.ts` — all 20 export paths change
2. `TerminalPanel.svelte` — update Terminal import to `"../features/Terminal.svelte"`
3. Zone → ContextBlock/Sparkline: all in `blocks/`, paths unchanged
4. `+page.svelte` imports from `$lib/components` barrel — no change

**Ordering:** Run after Subagent A completes (A touches +page.svelte, B touches component files — minimal conflict but cleaner sequentially).

---

### Subagent C: Performance Fixes

#### C1. Debounce localStorage writes

**Files:** `context.svelte.ts`, `zones.svelte.ts`, `editHistory.svelte.ts`

**Pattern (same for all three):**
```typescript
let dirty = $state(false);
let saveTimer: ReturnType<typeof setTimeout> | undefined;

function markDirty(): void { dirty = true; }

$effect(() => {
  if (!dirty) return;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveToLocalStorage();
    dirty = false;
  }, 1500);
});

function flushPendingWrites(): void {
  if (dirty) { saveToLocalStorage(); dirty = false; }
  if (saveTimer) { clearTimeout(saveTimer); saveTimer = undefined; }
}
```

**Replacements:**
- `context.svelte.ts`: 19 `saveToLocalStorage()` → `markDirty()`
- `zones.svelte.ts`: 16 → `markDirty()`
- `editHistory.svelte.ts`: 3 → `markDirty()`
- **Exception:** `init()`/`loadFromLocalStorage()` startup migrations keep direct `saveToLocalStorage()`

**Safety net:** Export `flushPendingWrites()` from each store. Call from `+page.svelte` `beforeunload` handler.

#### C2. Batch/Disable Transitions During Streaming

Add `batchMode` state to `ui.svelte.ts`. Zone.svelte accepts `transitionDuration` prop (default 150, 0 when batch mode).

#### C3. Replace JSON.parse(JSON.stringify()) with structuredClone()

**`context.svelte.ts`** + **`zones.svelte.ts`**: Replace all `JSON.parse(JSON.stringify(...))` instances with `structuredClone()`. Total: ~2 confirmed instances (may find more in snapshot save/switch/restore paths).

#### C4. Increase Search Debounce

**`search.svelte.ts`**: 150ms → 250ms.

---

### Subagent D: Backend Module Scaffolding

#### D1. Engine Module Skeleton

```
src-tauri/src/engine/
├── mod.rs     — pub mod block; pub mod types;
├── block.rs   — Universal Block struct (mirrors TS Block type), serde derives
└── types.rs   — Role, Zone, CompressionLevel, PinPosition enums
```

#### D2. Events Module Skeleton

```
src-tauri/src/events/
├── mod.rs     — pub mod types;
└── types.rs   — ApertureEvent enum (RequestCaptured, ResponseStreaming, ResponseComplete, ContextUpdated, ProxyError)
```

#### D3. Proxy Refactor — Split handler.rs

- `proxy/mod.rs` (trimmed to ~80 LOC): Keep startup, ProxyState, config
- `proxy/handler.rs` (new, ~200 LOC): Move `proxy_handler`, `forward_request`, `determine_upstream`, helpers, tests
- Add request UUID tracking + tracing span
- Add 120s upstream timeout on reqwest Client
- Add 3 new error variants: `RequestTooLarge`, `UpstreamTimeout`, `ParsingFailed`

#### D4. Register New Modules

`lib.rs`: Add `pub mod engine; pub mod events;`

#### D5. Add Dependency

`Cargo.toml`: Add `dashmap = "6"` (concurrent HashMap for Phase 1 multi-session state)

---

### Subagent E: Integration Verification (Sequential, after A-D)

1. `npm run check` — 0 errors
2. `npx vite build` — success
3. `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` — 0 warnings
4. `cargo fmt --manifest-path src-tauri/Cargo.toml --check` — passes
5. `cargo test --manifest-path src-tauri/Cargo.toml` — all pass
6. Verify +page.svelte script < 300 LOC
7. All component imports resolve from subdirectories
8. No `JSON.parse(JSON.stringify` remaining in stores
9. No direct `saveToLocalStorage()` in mutation functions (only in debounce effect + init)
10. Backend modules compile: `cargo check`

---

## Session 2: Documentation

### Subagent F: RESUME.md + FRONTEND_INVENTORY.md
- Phase 0 status → COMPLETE, add Phase 0.5 row
- Add missing components to FRONTEND_INVENTORY (ContextDiff, ZoneMinimap, Sparkline)
- Add composables section, update component paths to subdirectory structure

### Subagent G: Phase Docs + INTEGRATION.md
- Create `docs/INTEGRATION.md` — data flow diagram, Tauri IPC ref, events ref
- Update `phase-1.md` and `phase-2.md` to reference Phase 0.5 skeletons (extend, don't create)

### Subagent H: ARCHITECTURE.md + phase-0.5.md
- Update ARCHITECTURE.md with terminal, snapshots, new file structure
- Create `.context/phases/phase-0.5.md` — deliverables, verification, design decisions

---

## Execution Order

```
Subagent A (composables) ─────────┐
Subagent C (performance) ─────────┤── parallel
Subagent D (backend scaffolding) ─┘
         │
         ▼
Subagent B (component subdirs) ── after A completes
         │
         ▼
Subagent E (verification) ─────── after all complete
         │
         ▼
── CHECKPOINT: commit, evaluate context usage ──
         │
         ▼
Session 2: Documentation ─────── subagents F/G/H (won't bloat main context)
```

**Checkpoint note:** After Session 1 verification passes, commit the code changes. Then evaluate context usage — if healthy, proceed to Session 2 docs (all via subagents, minimal main context cost). If context is bloated, defer docs to next session.

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| No list virtualization yet | Mock data is small; implement in Phase 1 when real data flows |
| No persistence abstraction | Just debounce; full localStorage→SQLite abstraction in Phase 2 |
| structuredClone over delta snapshots | Quick native win now; delta encoding in Phase 2 |
| Tauri events over raw WebSocket | Phase 1 uses existing Tauri event system |
| Engine block.rs in Phase 0.5 | Skeleton only; establishes canonical location before Phase 1 |
| dashmap added early | Used immediately in Phase 1; no cost to add now |

---

## Verification

After all work:
1. `npm run check` — 0 errors
2. `npx vite build` — success
3. `cargo clippy -- -D warnings` — 0 warnings
4. `cargo fmt --check` — passes
5. `cargo test` — all pass
6. +page.svelte script section < 300 LOC
7. All component imports resolve from subdirectories
8. No direct `saveToLocalStorage()` in store mutation functions
9. No `JSON.parse(JSON.stringify` in stores
10. Backend modules compile (engine/, events/, split proxy/)
