# Phase 0.5: Foundation Hardening

> **Status:** COMPLETE
> **Duration:** 1 session
> **Prerequisite:** Phase 0 Complete
> **Leads into:** Phase 1 (Proxy Core)

---

## Context from Phase 0

Phase 0 delivered 20 components, 10 stores, an embedded terminal, snapshot branching, syntax highlighting, and more — but accumulated technical debt that would slow Phase 1. Five parallel audits scored the codebase 8.5/10 on structure but identified gaps in performance (5/10), code organization, and backend readiness.

**Audit commit:** `e756158` (initial fixes applied before this phase)

---

## Deliverables

### 1. Composable Extraction

Extracted ~600 LOC of handler logic from `+page.svelte` into `src/lib/composables/`:

| File | LOC | Responsibility |
|------|-----|----------------|
| `resizable.svelte.ts` | 265 | Sidebar, zone, terminal resize (rAF + direct DOM) |
| `commandHandlers.svelte.ts` | 220 | Full command palette dispatch |
| `keyboardHandlers.svelte.ts` | 177 | J/K/arrow nav, shortcuts, `allBlocksFlat` derived |
| `blockHandlers.svelte.ts` | 121 | Select, double-click, drag, context menu state |
| `modalHandlers.svelte.ts` | 78 | Compress, move, pin, remove, edit, role change |
| `index.ts` | 5 | Barrel re-export |

**Result:** +page.svelte script reduced from 752 LOC to 100 LOC.

Each composable exports a `create*()` factory that takes store/ref dependencies and returns state + handlers.

### 2. Component Subdirectories

Organized 20 flat components into logical groups:

```
src/lib/components/
├── blocks/     ContextBlock, Zone, Sparkline
├── layout/     Modal, TerminalPanel, TitleBar, ZoneManager
├── controls/   CommandPalette, ContextMenu, SearchBar, ThemeToggle, ThemeCustomizer, BlockTypeManager
├── features/   ContextDiff, Terminal, ZoneMinimap
├── ui/         CanvasOverlay, DensityControl, Toast, TokenBudgetBar
└── index.ts    Barrel re-exports (all paths updated)
```

### 3. Performance Fixes

| Fix | Details |
|-----|---------|
| Debounced localStorage | 38 direct `saveToLocalStorage()` → `markDirty()` + 1500ms timeout in context, zones, editHistory stores |
| `structuredClone()` | Replaced all `JSON.parse(JSON.stringify())` deep clones |
| Search debounce | 150ms → 250ms |
| Batch mode | `uiStore.batchMode` disables slide transitions during streaming (Zone `transitionDuration` prop) |
| `flushPendingWrites()` | Exported from all 3 stores, called on `beforeunload` |

### 4. Backend Module Scaffolding

```
src-tauri/src/
├── engine/
│   ├── mod.rs      pub mod block; pub mod types;
│   ├── block.rs    Universal Block struct (mirrors TS Block type)
│   └── types.rs    Role, Zone, CompressionLevel, PinPosition enums
├── events/
│   ├── mod.rs      pub mod types;
│   └── types.rs    ApertureEvent enum (5 event types)
├── proxy/
│   ├── mod.rs      Startup, ProxyState, config (~80 LOC)
│   ├── handler.rs  proxy_handler, forward_request, determine_upstream (~200 LOC)
│   └── error.rs    ProxyError + 3 new variants (RequestTooLarge, UpstreamTimeout, ParsingFailed)
└── lib.rs          pub mod engine; pub mod events; pub mod proxy; pub mod terminal;
```

**New dependency:** `dashmap = "6"` (concurrent HashMap for Phase 1 multi-session state)

---

## Verification Checklist

All 10 checks passed:

| # | Check | Result |
|---|-------|--------|
| 1 | `npm run check` — 0 errors | PASS |
| 2 | `npx vite build` — success | PASS |
| 3 | `cargo clippy -- -D warnings` — 0 warnings | PASS |
| 4 | `cargo fmt --check` — clean | PASS |
| 5 | `cargo test` — 3/3 pass | PASS |
| 6 | +page.svelte script < 300 LOC | PASS (100 LOC) |
| 7 | All component imports resolve | PASS (16/16) |
| 8 | No `JSON.parse(JSON.stringify` in stores | PASS |
| 9 | No direct `saveToLocalStorage()` in mutations | PASS |
| 10 | Backend modules compile | PASS |

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| No list virtualization yet | Mock data is small; implement in Phase 1 when real data flows |
| No persistence abstraction | Just debounce; full localStorage→SQLite in Phase 2 |
| structuredClone over delta snapshots | Quick native win; delta encoding in Phase 2 |
| `$state.snapshot()` over `structuredClone()` | `structuredClone()` throws on Svelte 5 `$state` proxies; `$state.snapshot()` recursively unwraps |
| Engine block.rs skeleton only | Establishes canonical location before Phase 1 fills it |
| dashmap added early | Used immediately in Phase 1; zero cost to add now |
| Composable factory pattern | Dependency injection via create*() keeps stores testable |
