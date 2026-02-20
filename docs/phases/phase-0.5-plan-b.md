# Phase 0.5: Foundation Hardening

## Context

Phase 0 massively overdelivered — 20 components, 10 stores, embedded terminal, snapshot branching, syntax highlighting, and more. Five parallel audits scored the codebase 8.5/10 on structure but found critical gaps in performance (5/10), documentation currency, and backend readiness for Phase 1. This plan hardens all foundations before wiring live proxy data.

**Key findings driving this work:**
- `+page.svelte` is 1,963 LOC (needs splitting into composables)
- 39 synchronous `saveToLocalStorage()` calls with zero debouncing
- `JSON.parse(JSON.stringify())` deep clones on every snapshot switch
- No backend module skeletons for engine or events (Phase 1 blockers)
- Documentation out of sync — FRONTEND_INVENTORY missing 3 components, phase docs describe work as "create" that already exists

---

## Execution Architecture

```
Session 1: Code Changes (parallel subagents)
  ├── Subagent A: Extract composables from +page.svelte
  ├── Subagent B: Component subdirectories + import updates
  ├── Subagent C: Performance fixes (debounce, structuredClone, transitions)
  ├── Subagent D: Backend module scaffolding + proxy refactor
  └── Subagent E: Integration verification (sequential, after A-D)

Session 2: Documentation (parallel subagents)
  ├── Subagent F: RESUME.md + FRONTEND_INVENTORY.md updates
  ├── Subagent G: Phase doc updates + create INTEGRATION.md
  └── Subagent H: ARCHITECTURE.md update + create phase-0.5.md
```

**Ordering constraint**: Subagents A and B both touch the component ecosystem. Run C+D in parallel with A, then B after A completes. E runs last.

---

## Subagent A: Extract Composables from +page.svelte

### Goal
Reduce script section from ~750 LOC to ~250 LOC by extracting handler logic into `src/lib/composables/`.

### Directory: `src/lib/composables/` (NEW)

### File 1: `resizable.svelte.ts` (~180 LOC)
Extract all three resize handler sets from +page.svelte:

**Sidebar resize** (lines 126-162): `handleResizeStart`, `handleResizeMove`, `handleResizeEnd`
- State: `isResizingSidebar`, `resizeStartX`, `resizeStartWidth`, `sidebarResizeRaf`, `sidebarRef`

**Zone resize** (lines 165-208): `handleZoneResizeStart`, `handleZoneResizeMove`, `handleZoneResizeEnd`
- State: `resizingZoneId`, `zoneResizeStartY`, `zoneResizeStartHeight`

**Terminal resize** (lines 211-304): `handleTermResizeStart`, `handleTermResizeMove`, `handleTermResizeEnd`
- State: `isResizingTerminal`, `termResizeStart`, `termResizeStartSize`, `termResizeRaf`, `terminalPanelRef`, `terminalPanelComponentRef`, `contentRef`
- Includes auto-collapse context panel logic + `MIN_CONTENT_SIZE` constant

**Exports**: `createResizable()` → returns all state + handlers as a single object

**Dependencies**: `uiStore`, `zonesStore`, `terminalStore` (imported within composable)

### File 2: `blockHandlers.svelte.ts` (~68 LOC)
Extract block event handlers from lines 448-516:

**Functions**: `handleBlockSelect`, `handleBlockDoubleClick`, `handleBlockDragStart`, `handleBlockDragEnd`, `handleBlockContextMenu`, `closeContextMenu`, `handleZoneDrop`, `handleZoneReorder`, `handleCreateBlock`, `handleToggleZoneCollapse`

**State**: `contextMenuBlock`, `contextMenuX`, `contextMenuY`, `contextMenuVisible`

**Exports**: `createBlockHandlers()` → state + all handlers

**Dependencies**: `contextStore`, `selectionStore`, `uiStore`, `zonesStore`, `blockTypesStore`

### File 3: `modalHandlers.svelte.ts` (~46 LOC)
Extract modal handlers from lines 518-564:

**Functions**: `handleModalClose`, `handleModalCompress`, `handleModalMove`, `handleModalPin`, `handleModalRemove`, `handleModalContentEdit`, `handleModalRoleChange`

**Exports**: `createModalHandlers()` → all handlers

**Dependencies**: `contextStore`, `uiStore`, `blockTypesStore`

### File 4: `keyboardHandlers.svelte.ts` (~133 LOC)
Extract keyboard navigation + shortcuts from lines 306-445:

**Functions**: `navigateBlock(direction)`, `handleKeydown(e)`
- Handles: Ctrl+T, Ctrl+F, F3, A, Esc, Del, S, K, [, J/K/↑/↓/Enter

**Derived state to move**: `allBlocksFlat` (line 307-309)

**Exports**: `createKeyboardHandlers(deps)` → `{ handleKeydown }`
- `deps` receives getter references to `terminalPanelRef` and `terminalPanelComponentRef` from resize composable

**Dependencies**: All stores (contextStore, selectionStore, uiStore, searchStore, terminalStore, zonesStore)

### File 5: `commandHandlers.svelte.ts` (~182 LOC)
Extract command palette dispatch from lines 567-748:

**Function**: `handleCommand(command: string)` — central dispatch for all command palette actions (view toggles, search, selection, edit, terminal, data commands)

**Exports**: `createCommandHandlers(deps)` → `{ handleCommand }`
- `deps` receives `terminalPanelComponentRef` getter and `openDiffView` callback

**Dependencies**: All stores

### File 6: `index.ts`
Barrel re-export of all five `create*` functions.

### Resulting +page.svelte
- Composable imports + instantiation: ~20 LOC
- Remaining state (diff view, snapshot CRUD): ~15 LOC
- `onMount` + `$effect` blocks: ~40 LOC
- Event wiring: ~20 LOC
- Template: ~484 LOC (unchanged)
- Style: ~726 LOC (unchanged)
- **Total script: ~250 LOC** (down from ~750)

---

## Subagent B: Component Subdirectories

### Goal
Organize 20 flat components into 5 logical subdirectories.

### Target Structure
```
src/lib/components/
├── blocks/          ContextBlock, Zone, Sparkline
├── layout/          Modal, TerminalPanel, TitleBar, ZoneManager
├── controls/        CommandPalette, ContextMenu, SearchBar, ThemeToggle, ThemeCustomizer, BlockTypeManager
├── features/        ContextDiff, ZoneMinimap, Terminal
├── ui/              CanvasOverlay, Toast, TokenBudgetBar, DensityControl
└── index.ts         (updated barrel)
```

### Moves (20 files via `git mv`)

| Component | From | To |
|-----------|------|----|
| ContextBlock | `./` | `blocks/` |
| Zone | `./` | `blocks/` |
| Sparkline | `./` | `blocks/` |
| Modal | `./` | `layout/` |
| TerminalPanel | `./` | `layout/` |
| TitleBar | `./` | `layout/` |
| ZoneManager | `./` | `layout/` |
| CommandPalette | `./` | `controls/` |
| ContextMenu | `./` | `controls/` |
| SearchBar | `./` | `controls/` |
| ThemeToggle | `./` | `controls/` |
| ThemeCustomizer | `./` | `controls/` |
| BlockTypeManager | `./` | `controls/` |
| ContextDiff | `./` | `features/` |
| ZoneMinimap | `./` | `features/` |
| Terminal | `./` | `features/` |
| CanvasOverlay | `./` | `ui/` |
| Toast | `./` | `ui/` |
| TokenBudgetBar | `./` | `ui/` |
| DensityControl | `./` | `ui/` |

### Import Updates Required
1. **`index.ts`** — All 20 export paths change (e.g., `"./Zone.svelte"` → `"./blocks/Zone.svelte"`)
2. **`Zone.svelte`** — Imports ContextBlock + Sparkline directly; all three in `blocks/`, paths stay as `"./ContextBlock.svelte"` — **no change**
3. **`TerminalPanel.svelte`** — Imports Terminal from `"./Terminal.svelte"`; update to `"../features/Terminal.svelte"`
4. **`+page.svelte`** — Imports from `$lib/components` barrel — **no change**
5. **No other files** import components directly (verified)

---

## Subagent C: Performance Fixes

### C1. Debounce localStorage Writes

**Files**: `context.svelte.ts`, `zones.svelte.ts`, `editHistory.svelte.ts`

**Pattern** (same for all three):
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

**Replacements**:
- `context.svelte.ts`: Replace 20 `saveToLocalStorage()` calls with `markDirty()`
- `zones.svelte.ts`: Replace 16 calls with `markDirty()`
- `editHistory.svelte.ts`: Replace 3 calls with `markDirty()`
- **Exception**: `init()`/`loadFromLocalStorage()` startup migrations keep direct `saveToLocalStorage()`

**Safety net**: Export `flushPendingWrites()` from each store. Call from `+page.svelte` `beforeunload` handler.

### C2. Batch/Disable Transitions During Streaming

**`src/lib/stores/ui.svelte.ts`**: Add `batchMode` state:
```typescript
let batchMode = $state(false);
function setBatchMode(enabled: boolean): void { batchMode = enabled; }
```

**`Zone.svelte`**: Change `transition:slide={{ duration: 150 }}` to accept a `transitionDuration` prop (default 150). `+page.svelte` passes `uiStore.batchMode ? 0 : 150`.

### C3. Replace JSON.parse(JSON.stringify()) with structuredClone()

**`context.svelte.ts`**: 6 replacements in snapshot save/switch/restore functions
**`zones.svelte.ts`**: 3 replacements in `captureState()`

Total: 9 replacements. `structuredClone` is faster (native, no string intermediate) and handles Date objects properly.

### C4. Increase Search Debounce

**`search.svelte.ts`**: Change debounce from `150` to `250` ms (line 108).

---

## Subagent D: Backend Module Scaffolding

### D1. Add Dependency
**`Cargo.toml`**: Add `dashmap = "6"` (concurrent HashMap for Phase 1 multi-session state)

### D2. Create Engine Module Skeleton

**`src-tauri/src/engine/mod.rs`**: Module declaration (`pub mod block; pub mod types;`)

**`src-tauri/src/engine/block.rs`**: Universal `Block` struct with serde derives. Fields mirror TypeScript Block type: id, role, block_type, content, tokens, timestamp, zone, pinned, compression_level, usage_heat, position_relevance, last_referenced_turn, reference_count, topic_cluster, topic_keywords.

**`src-tauri/src/engine/types.rs`**: Shared enums: `Role` (System/User/Assistant/ToolUse/ToolResult), `Zone` (Primacy/Middle/Recency/Custom), `CompressionLevel` (Original/Trimmed/Summarized/Minimal), `PinPosition` (Top/Bottom). All with serde derives.

### D3. Create Events Module Skeleton

**`src-tauri/src/events/mod.rs`**: Module declaration (`pub mod types;`)

**`src-tauri/src/events/types.rs`**: `ApertureEvent` enum (tagged, serde-serializable):
- `RequestCaptured { request_id, provider, endpoint, token_count }`
- `ResponseStreaming { request_id, chunk_index }`
- `ResponseComplete { request_id, token_count, duration_ms }`
- `ContextUpdated { block_count, total_tokens }`
- `ProxyError { request_id, message }`

### D4. Refactor Proxy — Split handler.rs

**`proxy/mod.rs`** (trimmed to ~80 LOC): Keep `start_proxy()`, `ProxyState`, `UpstreamConfig`, `DEFAULT_PORT`, `MAX_BODY_SIZE`. Move handler + helpers out. Add `pub mod handler;`.

**`proxy/handler.rs`** (new, ~200 LOC): Move `proxy_handler`, `forward_request`, `determine_upstream`, `convert_headers`, `log_headers`, and tests.

### D5. Expand Proxy Error Enum
Add to `proxy/error.rs`:
- `RequestTooLarge { size: usize, max: usize }`
- `UpstreamTimeout { duration_secs: u64 }`
- `ParsingFailed(String)`

### D6. Add Request ID Tracking
In `handler.rs` `proxy_handler`: Generate UUID per request, wrap handler in `tracing::info_span!("proxy_request", %request_id)`.

### D7. Add Upstream Timeout
In `ProxyState::new()`: Build reqwest Client with `.timeout(Duration::from_secs(120))`.

### D8. Register New Modules
In `lib.rs`: Add `pub mod engine; pub mod events;`

---

## Subagent E: Integration Verification

Sequential, after A-D complete:

1. `npm run check` — 0 errors
2. `npx vite build` — success
3. `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` — 0 warnings
4. `cargo fmt --manifest-path src-tauri/Cargo.toml --check` — passes
5. `cargo test --manifest-path src-tauri/Cargo.toml` — all pass
6. Verify +page.svelte script < 300 LOC
7. Verify all component imports resolve (build covers this)
8. Verify new Rust modules compile: `cargo check`
9. Grep: no `JSON.parse(JSON.stringify` remaining in stores
10. Grep: no direct `saveToLocalStorage()` in mutation functions (only in debounce effect + init)

---

## Session 2: Documentation

### Subagent F: RESUME.md + FRONTEND_INVENTORY.md

**RESUME.md changes**:
- Phase 0 status → `COMPLETE`
- Add Phase 0.5 row: `Foundation Hardening | COMPLETE`
- Add "Phase 0 Final Deliverables" section (20 components, 10 stores, all features)
- Add "Phase 0.5 Deliverables" section (composables, subdirs, perf, backend scaffolding)
- Update "Next Steps" → Phase 1 starts, proxy exists, engine skeleton ready

**FRONTEND_INVENTORY.md changes**:
- Add missing components: ContextDiff, ZoneMinimap, Sparkline (3 missing from table)
- Add editHistoryStore
- Add localStorage keys: `aperture-edit-history`, `aperture-minimap-visible`
- Update component paths to subdirectory structure
- Add "Composables" section (5 files)
- Update component count header

### Subagent G: Phase Docs + INTEGRATION.md

**Create `docs/INTEGRATION.md`** (NEW):
- ASCII data flow diagram (Proxy → Engine → UI)
- Tauri IPC command reference (6 current commands)
- Tauri events reference (2 current events)
- Planned events (Phase 1) — reference `events/types.rs`
- localStorage → backend migration strategy overview

**Update `.context/phases/phase-1.md`**:
- "Context from Phase 0" → add Phase 0.5 deliverables
- Clarify: `engine::block::Block` skeleton exists, extend don't create
- Clarify: `events::types::ApertureEvent` exists, extend don't create
- Clarify: proxy already has `handler.rs`, request ID, timeout
- Replace "WebSocket" references with "Tauri events" where appropriate

**Update `.context/phases/phase-2.md`**:
- Note engine/types skeletons exist from Phase 0.5
- Phase 2 extends, does not create
- Note localStorage debouncing as stepping stone to engine migration

### Subagent H: ARCHITECTURE.md + phase-0.5.md

**Update `docs/ARCHITECTURE.md`**:
- Add terminal to architecture diagram
- Add snapshot system to data model
- Update file structure to show: `composables/`, component subdirectories, `engine/`, `events/`, split `proxy/`
- Add `handler.rs` to proxy file listing

**Create `.context/phases/phase-0.5.md`** (NEW):
- Status: COMPLETE
- Problem statement (from audit findings)
- 6 deliverable areas with details
- Key files created/modified table
- Verification checklist (10 items)
- Design decisions with rationale
- Context for Phase 1

---

## File Manifest

### New Files (17)
| File | Agent | Purpose |
|------|-------|---------|
| `src/lib/composables/resizable.svelte.ts` | A | Resize handlers |
| `src/lib/composables/blockHandlers.svelte.ts` | A | Block event handlers |
| `src/lib/composables/modalHandlers.svelte.ts` | A | Modal handlers |
| `src/lib/composables/keyboardHandlers.svelte.ts` | A | Keyboard shortcuts |
| `src/lib/composables/commandHandlers.svelte.ts` | A | Command palette dispatch |
| `src/lib/composables/index.ts` | A | Barrel export |
| `src-tauri/src/engine/mod.rs` | D | Engine module |
| `src-tauri/src/engine/block.rs` | D | Block struct skeleton |
| `src-tauri/src/engine/types.rs` | D | Shared type enums |
| `src-tauri/src/events/mod.rs` | D | Events module |
| `src-tauri/src/events/types.rs` | D | Event enum |
| `src-tauri/src/proxy/handler.rs` | D | Proxy handler (split) |
| `docs/INTEGRATION.md` | G | Data flow + IPC ref |
| `.context/phases/phase-0.5.md` | H | Phase doc |
| `components/blocks/` | B | Subdirectory |
| `components/layout/` | B | Subdirectory |
| `components/controls/`, `features/`, `ui/` | B | Subdirectories |

### Modified Files (18)
| File | Agent | Changes |
|------|-------|---------|
| `src/routes/+page.svelte` | A | Extract ~550 LOC to composables |
| `src/lib/components/index.ts` | B | Update 20 export paths |
| `src/lib/components/layout/TerminalPanel.svelte` | B | Update Terminal import |
| `src/lib/stores/context.svelte.ts` | C | Debounce + structuredClone |
| `src/lib/stores/zones.svelte.ts` | C | Debounce + structuredClone |
| `src/lib/stores/editHistory.svelte.ts` | C | Debounce |
| `src/lib/stores/search.svelte.ts` | C | Increase debounce to 250ms |
| `src/lib/stores/ui.svelte.ts` | C | Add batchMode |
| `src/lib/components/blocks/Zone.svelte` | C | transitionDuration prop |
| `src-tauri/Cargo.toml` | D | Add dashmap |
| `src-tauri/src/lib.rs` | D | Register engine + events modules |
| `src-tauri/src/proxy/mod.rs` | D | Split handler out, add timeout |
| `src-tauri/src/proxy/error.rs` | D | 3 new variants |
| `.context/RESUME.md` | F | Phase 0 complete, 0.5 deliverables |
| `.context/FRONTEND_INVENTORY.md` | F | Missing components + stores |
| `.context/phases/phase-1.md` | G | Clarify existing modules |
| `.context/phases/phase-2.md` | G | Note 0.5 skeletons |
| `docs/ARCHITECTURE.md` | H | Terminal, snapshots, new structure |

### Moved Files (20 components via git mv)

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| **No list virtualization yet** | Mock data is small; implement in Phase 1 when real data flows |
| **No persistence abstraction** | Just debounce; full localStorage→SQLite abstraction in Phase 2 |
| **structuredClone over delta snapshots** | Quick native win now; delta encoding in Phase 2 |
| **Tauri events over raw WebSocket** | Phase 1 uses existing Tauri event system, no new WS dep needed |
| **Engine block.rs in Phase 0.5** | Skeleton only; establishes canonical location before Phase 1 parser needs it |
| **No CI/CD yet** | Phase 1 concern, not blocking |

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
11. RESUME.md shows Phase 0 COMPLETE
12. FRONTEND_INVENTORY.md lists all components
13. docs/INTEGRATION.md exists
