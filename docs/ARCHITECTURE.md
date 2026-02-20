# Aperture Architecture

> Universal LLM context visualization, management, and control proxy.

Last updated: 2026-02-19

---

## High-Level Overview

```
AI Coding Tool (Claude Code, Codex, etc.)
         │
         │  ANTHROPIC_BASE_URL / OPENAI_API_BASE
         │  pointed at localhost:5400
         ▼
┌──────────────────────────────────┐
│        Aperture Proxy            │
│                                  │
│  ┌────────────────────────────┐  │
│  │    Context Pipeline        │  │
│  │                            │  │
│  │  1. Ingest                 │  │
│  │  2. Parse to blocks        │  │
│  │  3. Zone assignment        │  │
│  │  4. Planner decisions      │  │
│  │  5. JSON rewriting         │  │
│  │  6. Budget/guardrails      │  │
│  │  7. Ingest + emit          │  │
│  └────────────────────────────┘  │
│                                  │
│  Modified request ──────────────►│──► Actual API
│                                  │
│  Response captured + forwarded   │──► Back to tool
└──────────────────────────────────┘
         │
         │  IPC / WebSocket
         ▼
┌──────────────────────────────────┐
│      Tauri Window (UI)           │
│                                  │
│  Live block visualization        │
│  Zone management                 │
│  Pipeline config                 │
│  Snapshots / Presets             │
└──────────────────────────────────┘
```

Security baseline for current and future phases: `docs/SECURITY_BASELINE.md`.

---

## Three Layers

### 1. Proxy Core (Rust / axum)

The HTTP proxy that intercepts all API traffic.

**Responsibilities:**
- Listen on localhost:5400
- Intercept requests from coding tools
- Forward to real API (Anthropic, OpenAI, etc.)
- Handle SSE streaming responses (tee to UI + client)
- Capture full request/response for engine

**Critical path:** Must be fast. Target <5ms overhead for non-LLM operations.

**Key modules:**
- `proxy/mod.rs` — server bootstrap and shared proxy state
- `proxy/handler.rs` + `proxy/handler/*` — request flow orchestration (routing/header filtering/exchange finalization helpers)
- `proxy/parser/*` — provider wire parsing and canonical block extraction
- `proxy/rewriter.rs` + `proxy/rewriter/*` — planner-driven JSON mutation/sanitization/injection
- `proxy/interceptor.rs` + `proxy/interceptor/*` — context-tool interception and bounded re-invoke flow
- `proxy/capture.rs` + `proxy/capture/*` — request/response capture store and SSE final-response reconstruction
- `proxy/context_api.rs` — internal `/_aperture/*` tool API
- `mcp/server.rs` — MCP JSON-RPC runtime used by `aperture_mcp` bin

### 2. Context Engine (Rust)

The brain that manages context state.

**Responsibilities:**
- Parse messages into universal Block format
- Assign blocks to zones (primacy/middle/recency)
- Track token counts per block
- Apply rules (auto-condensation, budget enforcement)
- Manage snapshots and presets
- Coordinate with cleaner model sidecar

**Key modules:**
- `engine/mod.rs` — engine entrypoints and public orchestration
- `engine/ingest.rs` — ingest lifecycle and regressive-capture protections
- `engine/session_sync.rs` — session persistence/sync/event helpers
- `engine/planner/*` — plan lifecycle, heuristic signals, mutation application, validation
- `engine/block.rs`, `engine/zone.rs`, `engine/budget.rs`, `engine/storage.rs` — core state, zoning, token budgeting, persistence

### 3. UI Layer (Svelte 5 / Tauri)

The visual interface.

**Responsibilities:**
- Real-time block visualization
- Drag-and-drop zone management
- Selection and bulk operations
- Pipeline configuration UI
- Canvas effects (halftone, dithering)

**Key directories:**
- `src/lib/components/` — Svelte components (organized into blocks/, layout/, controls/, features/, ui/)
- `src/lib/composables/` — Handler logic extracted from +page.svelte (resize, keyboard, block ops, modals, commands)
- `src/lib/stores/` — State management (Svelte 5 rune stores with debounced localStorage persistence)
- `src/lib/canvas/` — WebGL/Canvas effects
- `src/lib/utils/` — Shared utilities (text processing, syntax highlighting, diff)

**Embedded Terminal:**
- Frontend: xterm.js terminal emulator in `Terminal.svelte` / `TerminalPanel.svelte`
- Backend: portable-pty PTY session management in `src-tauri/src/terminal/`
- Communication via Tauri IPC commands and events (`terminal:output`, `terminal:exit`)
- Supports bottom/right positioning, snap-to-collapse, and theme synchronization

**Snapshot Branching System:**
- Working state + named snapshots with lineage tracking (`parentSnapshotId`)
- `activeSnapshotId`: null = working state, string = viewing a snapshot
- Zone state captured/restored alongside block state via `zonesStore.captureState()` / `restoreState()`
- Context diff view for comparing current state vs any snapshot (added/removed/modified blocks)

---

## Data Flow

### Request Flow

```
1. Claude Code sends request to localhost:5400
2. Proxy receives, parses message array
3. Engine processes through pipeline:
   - Parse → Blocks
   - Assign zones
   - Inject staged content
   - Apply auto-rules
   - Enforce budget
   - Reorder
4. Modified request sent to real API
5. Response streams back through proxy
6. UI updated via WebSocket
```

### UI Update Flow

```
1. Engine state changes (new block, selection, etc.)
2. Engine emits event via channel
3. Tauri IPC forwards to frontend
4. Svelte stores update
5. Components re-render
```

### Store Persistence

Svelte 5 rune stores use debounced localStorage persistence to avoid write thrashing.
Mutations call `markDirty()` which schedules a flush after 1500ms of inactivity.
This keeps the UI responsive during rapid state changes (drag operations, bulk edits)
while ensuring state survives page reloads.

---

## Universal Block Format

```rust
pub struct Block {
    pub id: String,
    pub role: Role,                    // system | user | assistant | tool_use | tool_result
    pub content: String,
    pub tokens: usize,
    pub timestamp: DateTime<Utc>,
    pub zone: Zone,                    // primacy | middle | recency
    pub pinned: Option<PinPosition>,   // top | bottom

    // Compression (non-destructive)
    pub compression_level: CompressionLevel,
    pub compressed_versions: CompressionVersions,

    // Heat & Attention
    pub usage_heat: f32,               // 0.0-1.0
    pub position_relevance: f32,       // 0.0-1.0
    pub last_referenced_turn: usize,
    pub reference_count: usize,

    // Clustering
    pub topic_cluster: Option<String>,
    pub topic_keywords: Vec<String>,

    // Metadata
    pub metadata: BlockMetadata,
}
```

---

## Provider Adapters

Each AI tool gets an adapter:

```rust
pub trait ProviderAdapter {
    fn name(&self) -> &str;
    fn detect_sessions(&self) -> Vec<Session>;
    fn parse_context(&self, session: &Session) -> Vec<Block>;
    fn watch(&self, session: &Session) -> Receiver<ContextEvent>;
    fn write_back(&self, session: &Session, blocks: &[Block]) -> Result<()>;
    fn api_base_url(&self) -> &str;
    fn transform_request(&self, req: ApiRequest) -> ApiRequest;
    fn transform_response(&self, res: ApiResponse) -> ApiResponse;
}
```

**Initial targets:**
- Claude Code (`~/.claude/projects/` JSONL)
- OpenAI-compatible (generic)

---

## Cleaner Model Sidecar

Background worker for LLM-powered tasks.

```
Aperture Engine
       │
       ├──► Proxy (fast path, zero model calls)
       │
       └──► Cleaner Sidecar (async background)
            │
            ├── Task Queue (priority-ordered)
            ├── Model Backend (ollama / llama.cpp)
            ├── Results Cache
            └── Health Monitor
```

**Tasks:**
- Pre-compute compression levels
- Attention echo analysis
- Semantic deduplication detection
- Topic classification
- Checkpoint boundary detection

**Key principle:** Never blocks the proxy. Results available on next request.

---

## IPC Protocol

Tauri IPC for frontend ↔ backend communication.

### Commands (Frontend → Backend)

```typescript
// Invoke Rust functions
await invoke('get_blocks');
await invoke('select_block', { id: '...' });
await invoke('compress_blocks', { ids: ['...'], level: 'summarized' });
await invoke('save_snapshot', { name: '...' });
```

### Events (Backend → Frontend)

```typescript
// Listen for state changes
listen('blocks_updated', (event) => { ... });
listen('selection_changed', (event) => { ... });
listen('token_count_changed', (event) => { ... });
```

---

## File Structure

```
src/
├── routes/
│   └── +page.svelte              # Main app page (~100 LOC script + template)
├── lib/
│   ├── components/
│   │   ├── blocks/               # ContextBlock, Zone, Sparkline
│   │   ├── layout/               # Modal, TerminalPanel, TitleBar, ZoneManager
│   │   ├── controls/             # CommandPalette, ContextMenu, SearchBar, ThemeToggle, ThemeCustomizer, BlockTypeManager
│   │   ├── features/             # ContextDiff, Terminal, ZoneMinimap
│   │   ├── ui/                   # CanvasOverlay, DensityControl, Toast, TokenBudgetBar
│   │   └── index.ts              # Barrel exports
│   ├── composables/              # Handler logic extracted from +page.svelte
│   │   ├── resizable.svelte.ts
│   │   ├── blockHandlers.svelte.ts
│   │   ├── modalHandlers.svelte.ts
│   │   ├── keyboardHandlers.svelte.ts
│   │   ├── commandHandlers.svelte.ts
│   │   └── index.ts
│   ├── stores/                   # Svelte 5 rune stores
│   ├── utils/                    # Shared utilities (text, syntax, diff)
│   ├── canvas/                   # Canvas effects (halftone, dissolution)
│   ├── types.ts                  # TypeScript interfaces
│   └── mock-data.ts              # Demo data generator

src-tauri/src/
├── lib.rs                        # Tauri app setup, module registration
├── main.rs                       # Entry point
├── proxy/                        # HTTP proxy (axum)
│   ├── mod.rs                    # Startup, ProxyState, config
│   ├── handler.rs                # Request routing, forwarding, SSE streaming
│   └── error.rs                  # ProxyError types
├── engine/                       # Context engine (Phase 1+)
│   ├── mod.rs
│   ├── block.rs                  # Universal Block struct
│   └── types.rs                  # Role, Zone, CompressionLevel enums
├── events/                       # Event system (Phase 1+)
│   ├── mod.rs
│   └── types.rs                  # ApertureEvent enum
└── terminal/                     # Embedded terminal (portable-pty)
    ├── mod.rs                    # Tauri commands, TerminalState
    ├── session.rs                # PTY session management
    └── error.rs                  # TerminalError types
```

---

## Performance Targets

| Operation | Target |
|-----------|--------|
| Proxy overhead (non-LLM) | <5ms |
| Block parsing | <1ms per block |
| Zone assignment | <100μs |
| UI update | <16ms (60fps) |
| Canvas render | <8ms |

---

## Security Considerations

- API keys pass through proxy — never logged, never stored
- All data stays local — no telemetry, no cloud sync
- Proxy binds to localhost only by default
- Optional encryption for snapshot storage
