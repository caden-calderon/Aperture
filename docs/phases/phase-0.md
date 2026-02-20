# Phase 0: UI Foundation

**Status**: READY FOR SPRINT
**Goal**: Complete Tauri + Svelte 5 app shell with full visual UI using mock data
**Prerequisites**: None
**Estimated Scope**: ~40-50k context (UI-only, no backend logic)

---

## Context

This is the foundation phase. We build the entire UI experience with mock data before touching any backend. The visual experience IS the product — if it doesn't feel incredible, nothing else matters.

**Reference**: `reference/context-forge-prototype.html` — Working HTML prototype showing core interactions

**⚠️ IMPORTANT: Use the `aperture-ui` skill for ALL UI work.**
The skill at `.claude/skills/aperture-ui.md` defines:
- Color system (CSS variables)
- Typography (JetBrains Mono, IBM Plex Mono — NO generic fonts)
- Halftone/dithering visual language
- Animation timings and physics
- Component patterns (Svelte 5 runes)

If the skill doesn't auto-activate, reference it manually. Every component must follow the design system.

---

## Problem Statement

1. **No app shell** — Need Tauri v2 + Svelte 5 project initialized
2. **No visual UI** — Need to translate prototype into production Svelte components
3. **No effects layer** — Need Canvas integration for halftone/dithering effects
4. **No animations** — Need the dissolution, materialization, and transition effects

---

## Deliverables

### 1. Tauri + Svelte Project Skeleton

- Tauri v2 app that launches
- Svelte 5 with SvelteKit
- Tailwind CSS configured
- TypeScript throughout
- Dev workflow: `pnpm tauri dev`

### 2. Core UI Components

| Component | Description |
|-----------|-------------|
| `TokenBudgetBar` | Top bar showing token usage by zone, dithered gradient |
| `ZoneContainer` | Primacy/Middle/Recency zone with collapsible header |
| `ContextBlock` | Individual block with type indicator, preview, tokens |
| `Sidebar` | Snapshots, block type filters, keyboard shortcuts |
| `Toolbar` | Selection actions (select all, condense, remove) |
| `CommandPalette` | Cmd+K palette for quick actions |

### 3. Interactions

- Click to select, shift+click for range select
- Drag blocks between zones
- Drag to trash zone (slides up from bottom)
- Double-click to expand block detail modal
- Keyboard shortcuts (A, Esc, Del, C, S)
- Pin cycling (primacy → recency → middle)

### 4. Visual Effects (Canvas Layer)

- Halftone dot patterns for heat visualization
- Dithered token budget bar
- Block dissolution animation (removal)
- Block materialization animation (appearance)
- Compression visual feedback (block shrinks, dithers)

### 5. Mock Data System

- Realistic demo context data
- Multiple block types (system, user, assistant, tool_use, tool_result)
- Simulated compression (UI only, no LLM)
- Snapshot save/restore (local state only)

### 6. Theming

- Dark mode (primary)
- CSS variables for all colors
- Halftone/dithering aesthetic throughout

---

## Key Files to Create

| File | Purpose |
|------|---------|
| `src/lib/components/TokenBudgetBar.svelte` | Token usage visualization |
| `src/lib/components/Zone.svelte` | Zone container |
| `src/lib/components/ContextBlock.svelte` | Individual block |
| `src/lib/components/Sidebar.svelte` | Left sidebar |
| `src/lib/components/Toolbar.svelte` | Selection actions |
| `src/lib/components/CommandPalette.svelte` | Cmd+K palette |
| `src/lib/components/Modal.svelte` | Block detail modal |
| `src/lib/components/Toast.svelte` | Notification toasts |
| `src/lib/canvas/halftone.ts` | Halftone rendering |
| `src/lib/canvas/effects.ts` | Dissolution/materialization |
| `src/lib/stores/context.ts` | Context state store |
| `src/lib/stores/selection.ts` | Selection state |
| `src/lib/stores/ui.ts` | UI state (modals, toasts) |
| `src/lib/types.ts` | TypeScript interfaces |
| `src/lib/mock-data.ts` | Demo data generator |

---

## Implementation Steps

### Step 1: Project Setup (~10k context)

1. Initialize Tauri v2 + Svelte 5 project
2. Configure Tailwind CSS
3. Set up TypeScript
4. Create basic app shell that launches
5. Verify `pnpm tauri dev` works

### Step 2: Layout & Core Components (~20k context)

1. Create main layout (header, sidebar, main area)
2. Build TokenBudgetBar component
3. Build Zone components (primacy, middle, recency)
4. Build ContextBlock component
5. Build Sidebar with sections
6. Wire up mock data

### Step 3: Interactions (~15k context)

1. Implement selection (click, shift+click, select all)
2. Implement drag-and-drop between zones
3. Implement trash zone
4. Implement keyboard shortcuts
5. Implement modal for block details
6. Implement toast notifications

### Step 4: Canvas Effects Layer (~15k context)

1. Set up Canvas overlay system
2. Implement halftone dot rendering
3. Implement dithered gradients for budget bar
4. Implement dissolution animation
5. Implement materialization animation
6. Implement compression visual feedback

### Step 5: Polish & Theming (~10k context)

1. Refine all animations and transitions
2. Ensure dark theme is consistent
3. Add hover states, focus states
4. Test keyboard navigation
5. Performance optimization

---

## Visual Reference

From `APERTURE.md` Aesthetic Direction:

- **Token budget bar**: Dithered gradient, dissolves from solid to scattered dots as limit approaches
- **Block heat**: Dot density = usage heat, dot color = position relevance
- **Compression**: Blocks visually "dissolve" — original=solid, minimal=almost gone
- **Zone borders**: Stippled/dithered lines
- **Colors**: Teal (primacy), yellow (middle), pink (recency)

---

## Success Criteria

- [x] `pnpm tauri dev` launches the app
- [x] All core components render correctly
- [x] Mock data displays in zones
- [x] Selection works (click, shift+click, keyboard)
- [x] Drag-and-drop works between zones
- [x] Animations are smooth (CSS transitions)
- [x] Halftone effects render correctly (CSS + Canvas)
- [x] Theme is consistent throughout (beige/black/cream)
- [x] Keyboard shortcuts work
- [x] Command palette works (Cmd+K)
- [x] `make check` passes

---

## Implementation Progress

*Updated: 2026-02-04*

### Step 1: Project Setup (COMPLETE)
- [x] Initialize Tauri v2 + Svelte 5 project
- [x] Configure Tailwind CSS (via @tailwindcss/vite)
- [x] TypeScript configured
- [x] Rust dependencies (axum, tokio, tiktoken-rs, etc.)
- [x] Basic UI shell renders
- [x] `npm run build` passes

### Step 2: Layout & Components (COMPLETE)
- [x] Main layout structure (header, sidebar, main area)
- [x] TokenBudgetBar with zone-segmented gradient, pressure states, dithered overlay
- [x] Zone components (primacy, middle, recency) with collapsible headers
- [x] ContextBlock component with role colors, heat overlay, compression visual
- [x] Sidebar (snapshots, filters, shortcuts)
- [x] Mock data system (`src/lib/mock-data.ts`)
- [x] Svelte 5 stores (`context.svelte.ts`, `selection.svelte.ts`, `ui.svelte.ts`)

### Step 3: Interactions (COMPLETE)
- [x] Selection (click, shift+click, ctrl+click range)
- [x] Drag-and-drop between zones
- [x] Keyboard shortcuts (A, Esc, Del, C, S, Cmd+K)
- [x] Modal for block details (with zone/compression/pin actions)
- [x] Toast notifications

### Step 4: Canvas Effects (COMPLETE)
- [x] CSS-based halftone patterns (dot overlays)
- [x] Pressure-based fill animations (pulse at high usage)
- [x] Canvas overlay system (`src/lib/canvas/`) with halftone utilities
- [x] Dissolution/materialization effect functions (ready for use)
- [x] TokenBudgetBar canvas-based halftone rendering
- [x] Compression visual feedback (scale + blur)

### Step 5: Polish (COMPLETE)
- [x] Animation basics (transitions, hover states)
- [x] Theme consistency (all CSS vars, JetBrains/IBM Plex fonts)
- [x] New beige/black/cream color palette (newspaper aesthetic)
- [x] CommandPalette component (Cmd+K)
- [x] `make check` passes (lint, typecheck, tests)
- [x] `npm run build` passes

---

## Pre-Phase 0: Proxy Validation (COMPLETE)

Proxy spike implemented and compiling:
- [x] Write minimal axum proxy in `src-tauri/src/proxy/`
- [x] SSE streaming passthrough implemented
- [x] Request/response logging with header redaction
- [x] Upstream routing (Anthropic vs OpenAI detection)
- [x] Unit tests passing (3/3)
- [ ] Live test with `ANTHROPIC_BASE_URL=http://localhost:5400` (needs `pnpm tauri dev`)

**Files created:**
- `src-tauri/src/proxy/mod.rs` — Main proxy implementation
- `src-tauri/src/proxy/error.rs` — Error types
- `tests/test_proxy.sh` — Manual test script

---

## Notes

- The HTML prototype (`reference/context-forge-prototype.html`) has working CSS that can be adapted
- Svelte 5 uses "runes" (`$state`, `$derived`, `$effect`) — different from Svelte 4
- Canvas layer sits behind/over DOM elements for per-pixel effects
- Consider `svelte-dnd-action` for drag-and-drop
