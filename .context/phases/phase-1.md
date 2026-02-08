# Phase 1: Proxy Core

**Status**: PENDING
**Goal**: Production proxy runtime that intercepts API calls, streams responses, and bridges live data into frontend stores
**Prerequisites**: Phase 0.5 complete
**Estimated Scope**: ~60-70k context (proxy + parsing + events + UI integration)

---

## Context from Phase 0 + Phase 0.5

Phase 0 delivers the complete visual UI with mock data:
- Tauri v2 + Svelte 5 app shell, 20 components in 5 subdirectories
- 6 composables extracted from +page.svelte (~100 LOC script)
- Selection, drag-drop, keyboard shortcuts, embedded terminal
- Snapshot branching, context diff, syntax highlighting
- Mock data system with localStorage persistence (debounced)

**Proxy already exists** (`src-tauri/src/proxy/`):
- `mod.rs` — ProxyState, UpstreamConfig, start_proxy() on port 5400
- `handler.rs` — proxy_handler, forward_request, determine_upstream (Anthropic/OpenAI auto-detection)
- `error.rs` — ProxyError with RequestTooLarge, UpstreamTimeout, ParsingFailed variants
- SSE streaming passthrough, request/response logging, unit tests passing

**Backend skeletons exist** (from Phase 0.5):
- `engine/block.rs` — Universal Block struct with all fields + serde derives
- `engine/types.rs` — Role, Zone, CompressionLevel, PinPosition enums
- `events/types.rs` — ApertureEvent enum with 5 variants + channel constants
- `dashmap` dependency already added to Cargo.toml

**Key imports:**
```rust
use crate::proxy::{ProxyState, start_proxy};
use crate::proxy::error::ProxyError;
use crate::engine::block::Block;
use crate::engine::types::{Role, Zone, CompressionLevel};
use crate::events::types::{ApertureEvent, channels};
```

**Integration point:** Phase 1 extends the existing proxy and skeletons to connect with the UI.

---

## Problem Statement

1. **No live data flow** — UI shows mock data, not real API traffic
2. **No request capture** — Proxy forwards but doesn't extract context for engine
3. **No UI updates** — No stable event bridge to push state changes to frontend stores
4. **No provider detection** — Need to auto-detect Anthropic vs OpenAI from requests
5. **No Responses API support** — Codex/OpenAI flows may use `/v1/responses`, not only chat completions
6. **No seamless in-app tool launch** — Launching Claude/Codex from terminal should auto-connect to Aperture without manual env setup each time
7. **No pause/hold** — Can't intercept requests for inspection before forwarding

---

## Deliverables

### 1. Enhanced Proxy Server

Extend `src-tauri/src/proxy/` to:
- Capture full request/response message arrays
- Emit structured events for engine consumption
- Support pause/hold mode for request inspection
- Handle all Anthropic API endpoints (`/v1/messages`, `/v1/complete`)
- Handle OpenAI-compatible endpoints (`/v1/chat/completions`, `/v1/responses`)

### 2. Request/Response Parsing

Create `src-tauri/src/proxy/parser.rs`:
- Parse Anthropic message format into blocks
- Parse OpenAI Chat Completions format into blocks
- Parse OpenAI Responses format into blocks
- Normalize both into universal Block format (use `engine::block::Block`)
- Handle tool_use and tool_result blocks
- Extract token counts from responses

**Note:** The canonical `Block` struct already exists in `engine::block.rs` (created in Phase 0.5). Phase 1's parser uses it directly: `use crate::engine::block::Block`.

### 3. Event System

Extend `src-tauri/src/events/` (skeleton exists from Phase 0.5):
- `types.rs` already defines ApertureEvent enum + channel constants — extend as needed
- Add event dispatcher (broadcasts to frontend via Tauri events)
- Add strongly typed payloads for store consumption
- Keep WebSocket optional for external consumers (defer if not required for core app)

### 4. Frontend Integration

Update Svelte stores:
- `src/lib/stores/context.svelte.ts` — Subscribe to backend events
- Replace mock data with live data from proxy
- Show real-time streaming indicator during responses
- Handle connection state (connected/disconnected/reconnecting)

### 5. Provider Auto-Detection

Detect provider from request characteristics:
- `x-api-key` header → Anthropic
- `Authorization: Bearer` → OpenAI
- Path patterns (`/v1/messages` vs `/v1/chat/completions` vs `/v1/responses`)
- Store detected provider per session

Detection requirement:
- Do not rely on provider-specific key prefixes (`sk-*`) to detect OpenAI traffic.
- Treat valid bearer auth + OpenAI endpoint patterns as sufficient for routing.

### 6. Seamless In-App Provider Launch

When launched from Aperture's embedded terminal, provider CLIs should auto-connect:
- `claude` / `claude-code` launched with `ANTHROPIC_BASE_URL=http://localhost:5400`
- `codex` launched with `OPENAI_BASE_URL` and `OPENAI_API_BASE` pointing to proxy
- Auto-tag session metadata with launch source (`embedded-terminal`) and provider
- Show connection indicator in UI once first request is observed

Goal:
- Open app → open terminal → run `claude` or `codex` → context appears with no extra manual wiring

### 6a. Provider Selector Quick Launch

Add a one-click launch flow in UI:
- Provider selector (Claude Code | Codex) in terminal controls/header
- "Launch" action starts selected CLI with proxy env vars injected
- Show launch status (`starting`, `connected`, `error`) and selected provider
- Keep manual terminal usage available; selector is additive convenience

### 7. Hot Patch Mode

Allow edits to take effect on the next request:
- Store pending block modifications in proxy state
- On next outbound request, apply pending edits before forwarding
- Clear pending edits after application
- Track edit source (manual, auto-rule) for versioning

**Note:** Hot patch edits don't block the current request — they queue for the next one. This enables "fix while working" without pausing.

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/proxy/parser.rs` | **NEW** | Message array → Block parsing |
| `src-tauri/src/proxy/capture.rs` | **NEW** | Request/response capture logic |
| `src-tauri/src/proxy/mod.rs` | Modify | Integrate capture + events |
| `src-tauri/src/proxy/handler.rs` | Modify | Add capture hooks (exists from Phase 0.5) |
| `src-tauri/src/events/mod.rs` | Modify | Extend with dispatcher (exists from Phase 0.5) |
| `src-tauri/src/events/types.rs` | Modify | Extend event variants as needed (exists from Phase 0.5) |
| `src-tauri/src/events/dispatcher.rs` | **NEW** | Event broadcasting to frontend |
| `src-tauri/src/terminal/mod.rs` | Modify | Add provider-aware launch helpers (Claude/Codex env injection) |
| `src/lib/components/layout/TerminalPanel.svelte` | Modify | Provider selector + quick launch controls |
| `src/lib/stores/context.svelte.ts` | Modify | Tauri event subscription |
| `src/lib/stores/connection.svelte.ts` | **NEW** | Connection state management |

---

## Implementation Steps

### Step 1: Request/Response Parsing (~15k context)

1. Define `ParsedRequest` and `ParsedResponse` types
2. Implement Anthropic message parser
3. Implement OpenAI Chat Completions parser
4. Implement OpenAI Responses parser
5. Implement normalization to universal Block format
6. Unit tests for all parsers

### Step 2: Capture System (~10k context)

1. Create capture middleware for axum
2. Extract message arrays from requests
3. Extract content + tokens from responses
4. Handle SSE streaming (accumulate chunks for parsing)
5. Unit tests for capture

### Step 3: Event System (~15k context)

1. Set up Tauri event dispatcher alongside HTTP proxy capture path
2. Define event types with serde serialization
3. Create event dispatcher (broadcasts to UI listeners)
4. Add optional WebSocket bridge only if external client support is required
5. Unit tests for event serialization

### Step 4: Frontend Integration (~10k context)

1. Subscribe stores to Tauri events
2. Update context store to receive live events
3. Add connection status indicator to UI
4. Handle reconnection logic
5. Test with real API call through proxy

### Step 5: Embedded Terminal Launch Integration (~8k context)

1. Add terminal launch helpers for Claude and Codex
2. Inject proxy base-url env vars for each provider CLI
3. Add provider selector + quick launch controls in UI
4. Emit launch metadata event for session labeling
5. Verify first request auto-associates with launched provider
6. Unit/integration tests for launch env injection and selector behavior

---

## Test Coverage

### Unit Tests (~25 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/proxy/parser.rs` | 12 | Anthropic + OpenAI chat/responses parsing |
| `src-tauri/src/proxy/capture.rs` | 8 | Request/response capture |
| `src-tauri/src/events/types.rs` | 4 | Event serialization |
| `src-tauri/src/events/dispatcher.rs` | 3 | Event dispatch basics |
| `src-tauri/src/terminal/mod.rs` | 3 | Provider launch env injection |
| `src/lib/components/layout/TerminalPanel.svelte` | 2 | Provider selector state + launch actions |

### Integration Tests (~8 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_proxy_flow.rs` | 5 | Full request → parse → event flow |
| `tests/integration/test_event_bridge.rs` | 3 | Tauri event bridge + store updates |
| `tests/integration/test_provider_launch.rs` | 2 | Embedded terminal launch wiring |

### Manual Tests (8 tests)

| Test | Description |
|------|-------------|
| `test_anthropic_passthrough` | Real Claude API call through proxy |
| `test_openai_passthrough` | Real OpenAI API call through proxy |
| `test_openai_responses_passthrough` | Real OpenAI Responses API call through proxy |
| `test_sse_streaming` | Verify streaming responses display in UI |
| `test_ui_live_update` | Verify UI updates when request captured |
| `test_reconnection` | Restart proxy/event listeners, verify state recovery |
| `test_pause_mode` | Enable pause, verify request held until released |
| `test_embedded_terminal_launch` | Launch `claude`/`codex` in terminal panel, verify auto-connect and provider tagging |
| `test_provider_selector_launch` | Select provider, click launch, verify CLI starts with correct env wiring |

---

## Success Criteria

- [ ] Proxy intercepts and forwards Anthropic API calls
- [ ] Proxy intercepts and forwards OpenAI-compatible API calls
- [ ] Proxy intercepts and forwards OpenAI Responses API calls (`/v1/responses`)
- [ ] Message arrays parsed into universal Block format
- [ ] UI receives real-time updates via event bridge
- [ ] Streaming responses show progress indicator
- [ ] Connection status visible in UI
- [ ] In-app terminal launch of `claude`/`codex` auto-connects to proxy
- [ ] Provider selector quick-launch works for Claude Code and Codex
- [ ] Pause mode holds request until manual release
- [ ] Provider auto-detected from request headers
- [ ] `make check` passes
- [ ] 25+ unit tests passing
- [ ] 8+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::proxy::{ProxyServer, ProxyConfig, CapturedRequest, CapturedResponse};
use crate::proxy::parser::{parse_anthropic, parse_openai, Block};
use crate::events::{EventDispatcher, ContextEvent};
```

```typescript
import { contextStore, connectionStore } from '$lib/stores';
import type { Block, ContextEvent } from '$lib/types';
```
