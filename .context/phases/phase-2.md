# Phase 2: Context Engine

**Status**: PENDING
**Goal**: Core engine for block/session management, persistence, classification, and policy-safe context actions
**Prerequisites**: Phase 1 + Phase 1.5 complete
**Estimated Scope**: ~55k context

---

## Context from Phase 1 + Phase 1.5

Phase 1 delivers:
- Proxy that intercepts and forwards API calls
- Request/response parsing into universal Block format
- Event bridge for real-time UI updates
- Provider auto-detection (Anthropic vs OpenAI)
- Pause/hold mode for request inspection

Phase 1.5 hardening adds:
- Provider adapter boundaries for launch/parser/capabilities
- Manual/quick-launch lifecycle convergence for Claude/Codex paths
- Codex direct bridge polling hardening and instrumentation
- Frontend build chunking guardrails for future growth

**Key imports:**
```rust
use crate::proxy::parser::{parse_anthropic, parse_openai, Block};
use crate::events::{EventDispatcher, ContextEvent};
```

**Integration point:** Phase 2 builds the engine that processes parsed blocks.

---

## Problem Statement

1. **No zone management** — Blocks parsed but not assigned to primacy/middle/recency
2. **No accurate token counting** — Need tiktoken integration per model
3. **No staleness tracking** — Can't identify old/unused blocks
4. **No sorting pipeline** — No automatic classification based on content/age
5. **No session management** — Can't track multiple concurrent sessions
6. **No edit tracking** — Users can modify blocks but no undo/history
7. **No dependency awareness** — Don't know which blocks reference others
8. **No budget alerts** — No warnings when approaching token limit
9. **No action policy layer** — No centralized guardrails/undo log for automated mutations

---

## Deliverables

### 1. Block Management System

Extend `src-tauri/src/engine/` (skeleton exists from Phase 0.5):
- `block.rs` already defines canonical Block struct with all fields + serde derives
- `types.rs` already defines Role, Zone, CompressionLevel, PinPosition enums
- Add in-memory block storage with efficient lookup (`store.rs`)
- CRUD operations for blocks
- Batch operations (select multiple, compress multiple)
- Block versioning (track edits)

### 2. Zone System

Implement the three-zone model:
- **Primacy** — System prompts, pinned critical content
- **Middle** — Default zone, conversation history
- **Recency** — Last N turns, latest tool outputs

Auto-assignment rules:
- System role → primacy
- Last 5 turns → recency
- Everything else → middle
- Manual pinning overrides auto-assignment

### 3. Token Counting

Accurate per-model token counting:
- Integrate tiktoken-rs for OpenAI models
- Handle Anthropic tokenization (claude-3 tokenizer)
- Cache token counts per block
- Update UI token displays in real-time

### 4. Staleness Scoring

Track block freshness:
```
staleness = (turns_since_created × token_cost) / relevance_boost
```

- `turns_since_created` — How old is this block
- `token_cost` — Higher cost = more pressure to remove
- `relevance_boost` — References from later blocks reduce staleness

### 5. Context Sorting Pipeline

Fast classification pipeline (millisecond-scale, deterministic):

**Pass 1 — Role-based (instant):**
- System prompts → primacy
- Last N turns → recency
- Everything else → middle

**Pass 2 — Content heuristics (fast):**
- Error/traceback → boost toward recency
- Old file reads → demote in middle
- Correction patterns ("actually", "wait") → flag preceding block

**Pass 3 — Budget-aware ranking:**
- Approaching limit → rank middle zone by staleness
- Flag worst offenders for compression/removal

### 6. Session Management

Track multiple concurrent sessions:
- Session ID per active tool connection
- Separate block stores per session
- Session metadata (provider, start time, token budget)
- Session switching in UI

### Storage Layer (SQLite)

Persistent storage for sessions, blocks, and metadata:
- **Location:** `~/.aperture/aperture.db`
- **Schema:** Sessions, blocks, checkpoints, settings tables
- **Why SQLite:**
  - Single file, easy backup
  - Full-text search (FTS5) for Phase 8
  - Structured queries (filter by date, session, block type)
  - Rust support via `rusqlite`
  - Can store embeddings efficiently (BLOB)

**Initial tables:**
- `sessions` — id, provider, start_time, token_budget, metadata
- `blocks` — id, session_id, role, content, tokens, zone, compression_level, timestamps
- `block_versions` — block_id, version, content, edited_at, edit_source

**Note:** In-memory store (`BlockStore`) is the hot path for current session. SQLite syncs in background for persistence and historical access.

### 7. Basic Block Versioning

Track edits to blocks (foundation for Phase 11's advanced versioning):
- Store edit history per block (last 5 versions)
- Simple undo for manual edits
- Track who/what made the change (user, auto-rule, hot-patch)

### 8. Basic Dependency Tracking

Deterministic dependency tracking (Phase 7 adds semantic deps):
- File reference chains (block reads file → later blocks reference that file)
- Conversation flow (user → assistant → tool chains)
- Simple orphan detection (block's referenced file was removed)

### 9. Basic Budget Alerts

Simple threshold warnings (Phase 9 adds advanced analytics):
- Alert at 80%, 90%, 95% budget utilization
- Visual indicator on token budget bar
- Optional toast notification

### 10. Policy + Action Log Foundation

Add a deterministic mutation layer used by later phases:
- Central policy checks before destructive actions (compress/archive/remove)
- Structured action log with actor/reason/inputs/results
- Undo/rollback hooks for reversible operations
- Event payloads for transparent UI history

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/engine/mod.rs` | Modify | Extend module root (exists from Phase 0.5) |
| `src-tauri/src/engine/block.rs` | Modify | Extend Block struct + add operations (exists from Phase 0.5) |
| `src-tauri/src/engine/types.rs` | Modify | Extend type enums as needed (exists from Phase 0.5) |
| `src-tauri/src/engine/store.rs` | **NEW** | In-memory block storage |
| `src-tauri/src/engine/zone.rs` | **NEW** | Zone management |
| `src-tauri/src/engine/tokens.rs` | **NEW** | Token counting |
| `src-tauri/src/engine/staleness.rs` | **NEW** | Staleness scoring |
| `src-tauri/src/engine/pipeline.rs` | **NEW** | Sorting/classification |
| `src-tauri/src/engine/session.rs` | **NEW** | Session management |
| `src-tauri/src/engine/versioning.rs` | **NEW** | Basic edit history |
| `src-tauri/src/engine/dependency.rs` | **NEW** | Deterministic dep tracking |
| `src-tauri/src/engine/budget.rs` | **NEW** | Budget threshold alerts |
| `src-tauri/src/engine/policy.rs` | **NEW** | Mutation policy checks |
| `src-tauri/src/engine/action_log.rs` | **NEW** | Audit/undo action log |
| `src-tauri/src/engine/storage.rs` | **NEW** | SQLite persistence layer |
| `src-tauri/src/commands.rs` | Modify | Add engine IPC commands |
| `src/lib/stores/context.svelte.ts` | Modify | Use real engine data |
| `src-tauri/Cargo.toml` | Modify | Add `rusqlite` dependency |

---

## Implementation Steps

### Step 1: Block & Store (~15k context)

1. Extend existing Block struct (already defined in `engine/block.rs` from Phase 0.5) with any additional fields needed
2. Implement BlockStore with HashMap-based storage (using `dashmap` already in Cargo.toml)
3. CRUD operations with proper event emission
4. Batch operations (get_many, update_many)
5. Unit tests for store operations

### Step 2: Zone System (~12k context)

1. Define Zone enum and ZoneConfig
2. Implement zone assignment logic
3. Implement pinning (primacy_top, primacy_bottom, recency_top, recency_bottom)
4. Implement zone reordering
5. Unit tests for zone assignment

### Step 3: Token Counting (~10k context)

1. Integrate tiktoken-rs
2. Create model-aware tokenizer selection
3. Cache token counts in blocks
4. Add token recalculation on content change
5. Unit tests for various models

### Step 4: Staleness & Pipeline (~10k context)

1. Implement staleness scoring algorithm
2. Implement three-pass classification pipeline
3. Add content heuristics (error detection, correction patterns)
4. Performance optimization (target <2ms per classification cycle)
5. Unit tests for pipeline stages

### Step 5: Session Management (~8k context)

1. Define Session struct
2. Implement session store (multiple concurrent sessions)
3. Add session switching logic
4. Integrate with proxy (create session on first request)
5. UI integration for session picker

### Step 6: Versioning, Dependencies, Budget & Policy (~10k context)

1. Implement basic block versioning (last 5 edits)
2. Implement deterministic dependency tracking (file refs, conversation flow)
3. Add simple orphan detection
4. Implement budget threshold alerts (80%, 90%, 95%)
5. Implement policy checks + structured action logging
6. Unit tests for versioning, deps, budget, and policy

---

## Test Coverage

### Unit Tests (~50 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/engine/block.rs` | 8 | Block creation, modification |
| `src-tauri/src/engine/store.rs` | 10 | Store CRUD, batch ops |
| `src-tauri/src/engine/zone.rs` | 8 | Zone assignment, pinning |
| `src-tauri/src/engine/tokens.rs` | 6 | Token counting accuracy |
| `src-tauri/src/engine/staleness.rs` | 4 | Staleness calculation |
| `src-tauri/src/engine/pipeline.rs` | 4 | Pipeline classification |
| `src-tauri/src/engine/versioning.rs` | 4 | Edit history, undo |
| `src-tauri/src/engine/dependency.rs` | 4 | File refs, orphan detection |
| `src-tauri/src/engine/budget.rs` | 2 | Threshold alerts |
| `src-tauri/src/engine/policy.rs` | 2 | Mutation policy checks |
| `src-tauri/src/engine/action_log.rs` | 2 | Action log/undo records |

### Integration Tests (~10 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_engine_flow.rs` | 6 | Parse → store → classify flow |
| `tests/integration/test_session.rs` | 4 | Multi-session handling |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_zone_display` | Verify blocks appear in correct zones in UI |
| `test_token_accuracy` | Compare token counts with official API |
| `test_staleness_visual` | Verify stale blocks visually dim |
| `test_multi_session` | Switch between sessions, verify state |
| `test_pinning` | Pin block to zone, verify it stays |
| `test_large_context` | Load 100+ blocks, verify performance |

---

## Success Criteria

- [ ] Blocks stored and retrievable by ID
- [ ] Zones auto-assigned based on role and position
- [ ] Manual pinning works and overrides auto-assignment
- [ ] Token counts accurate within 2% of official API
- [ ] Staleness scores calculated and displayed
- [ ] Classification pipeline runs in <2ms
- [ ] Multiple sessions tracked concurrently
- [ ] Session switching works in UI
- [ ] Basic block versioning with undo works
- [ ] Deterministic dependency tracking detects file references
- [ ] Budget alerts trigger at 80%, 90%, 95%
- [ ] Policy checks gate destructive actions with logged reasons
- [ ] `make check` passes
- [ ] 50+ unit tests passing
- [ ] 10+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::engine::{Engine, Block, BlockStore, Zone, Session};
use crate::engine::tokens::count_tokens;
use crate::engine::pipeline::classify_blocks;
use crate::engine::staleness::calculate_staleness;
use crate::engine::versioning::{BlockVersion, VersionHistory};
use crate::engine::dependency::{DependencyGraph, DependencyEdge};
use crate::engine::budget::{BudgetAlert, check_budget_thresholds};
use crate::engine::policy::PolicyEngine;
use crate::engine::action_log::{ActionLog, ActionRecord};
```

---

## Notes

**Block type definition:** The universal `Block` struct is defined in `engine::block.rs` and re-exported by `proxy::parser` for convenience. There is one canonical Block type shared across the codebase.

**Versioning scope:** Phase 2 implements basic edit history (last 5 versions, simple undo). Phase 11 enhances this with adaptive learning analysis and correction pattern tracking.

**Dependency scope:** Phase 2 implements deterministic tracking (file refs, conversation chains). Phase 7 adds semantic dependencies detected by the cleaner model.
