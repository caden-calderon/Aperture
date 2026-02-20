# Phase 5: Memory Lifecycle, Checkpoints & Forking

**Status**: PENDING
**Goal**: Non-destructive memory lifecycle (hot/warm/cold), archive/recall, checkpoints, forking, ghost blocks, and trash recovery
**Prerequisites**: Phase 4 complete
**Estimated Scope**: ~55k context

---

## Context from Phase 4

Phase 4 delivers:
- Usage heat tracking (how often blocks referenced)
- Position relevance scoring (primacy/recency attention)
- Heat visualization (dot density + color)
- Topic clustering with keyword overlap
- Semantic deduplication detection
- Heat-based recommendations

**Key imports:**
```rust
use crate::engine::heat::{calculate_heat, HeatMap};
use crate::engine::clustering::{cluster_blocks, TopicCluster};
use crate::engine::dedup::find_duplicates;
```

**Integration point:** Phase 5 adds state management and recovery capabilities.

---

## Problem Statement

1. **No memory lifecycle** — No hot/warm/cold/archive state model
2. **No recall path** — Archived context cannot be quickly recovered by topic/query
3. **No state snapshots** — Can't save and restore context state
4. **No experimentation** — Can't branch context for A/B exploration
5. **No deletion recovery** — Removed blocks are gone forever
6. **No conversational flow** — Removed blocks leave unexplained gaps

---

## Deliverables

### 1. Memory Lifecycle State Machine

Introduce deterministic lifecycle transitions:
- `hot` (expanded, in active prompt window)
- `warm` (compressed but still in active context)
- `cold` (out of active context, quick recall metadata retained)
- `archived` (offloaded, recallable, not deleted)

Rules:
- Keep originals preserved locally at all times
- Transition by policy/action log (from Phase 2), never silent mutation
- Every transition is reversible (undo or recall)

### 2. Context Manifest + Recall

Generate a compact manifest to expose current memory state:
- Block/zone counts, token budget, compressed/archived counts
- Active topic focus + recent transitions
- Recall hints (`topic`, `file`, `block-id`)

Recall primitives:
- `recall(topic)`
- `recall(block-id)`
- `recall(query)` for keyword or semantic recall (Phase 8 improves retrieval)

### 3. Hard Checkpoints

Exact byte-for-byte snapshots:
- Full context state at a moment in time
- Stored locally (size doesn't matter)
- Instant restore to exact state
- Use case: "About to try something risky, save state"

```rust
pub struct HardCheckpoint {
    pub id: String,
    pub name: String,
    pub timestamp: DateTime<Utc>,
    pub blocks: Vec<Block>,
    pub session_state: SessionState,
    pub size_bytes: usize,
}
```

### 4. Soft Checkpoints

Intelligent summaries for cross-session use:
- Condensed conversation summary
- Files read + key findings
- Current task status
- Decisions made + rationale
- Smaller, portable, injectable into primacy

Use case: "Done for today, capture where I left off"

```rust
pub struct SoftCheckpoint {
    pub id: String,
    pub name: String,
    pub timestamp: DateTime<Utc>,
    pub summary: String,
    pub files_referenced: Vec<FileReference>,
    pub task_status: Option<String>,
    pub decisions: Vec<Decision>,
    pub tokens: usize,
}
```

### 5. Auto-Checkpointing

Automatic saves at meaningful boundaries:
- Subtask completion (detected via TODO patterns)
- Successful file writes
- Test runs (pass/fail)
- Topic shifts (cluster change)
- Configurable triggers

### 6. Context Forking

Branch context at decision points:
- Fork creates independent copy
- Each branch has own state
- Compare branches side-by-side
- Merge winner back to main
- Visual branch indicator in UI

Use case: "WebSockets vs SSE?" — fork, explore both, pick winner

### 7. Ghost Blocks

Minimal placeholders for removed blocks:
```
Ghost: [config.py was read here — 847 tokens, removed at 2:34 PM]
```

- ~10-15 tokens cost
- Preserves conversational flow
- Prevents model disorientation
- Visually distinct (heavy dither, barely there)
- Can expand to compressed version if in trash

### 8. Recently Deleted (Trash)

Recovery system for removed blocks:
- All removals go to trash first
- Configurable retention (hours, sessions, or manual purge)
- Browse, search, filter trash
- One-click restore to original position
- Bulk restore
- Size indicator ("42 blocks, 12,400 tokens recoverable")

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/engine/memory.rs` | **NEW** | Lifecycle state machine + transitions |
| `src-tauri/src/engine/manifest.rs` | **NEW** | Context manifest generation |
| `src-tauri/src/engine/recall.rs` | **NEW** | Archive recall API |
| `src-tauri/src/engine/checkpoint.rs` | **NEW** | Checkpoint system |
| `src-tauri/src/engine/checkpoint/hard.rs` | **NEW** | Hard checkpoint logic |
| `src-tauri/src/engine/checkpoint/soft.rs` | **NEW** | Soft checkpoint generation |
| `src-tauri/src/engine/checkpoint/auto.rs` | **NEW** | Auto-checkpoint triggers |
| `src-tauri/src/engine/fork.rs` | **NEW** | Forking system |
| `src-tauri/src/engine/ghost.rs` | **NEW** | Ghost block generation |
| `src-tauri/src/engine/trash.rs` | **NEW** | Recently deleted storage |
| `src/lib/components/CheckpointPanel.svelte` | **NEW** | Checkpoint management UI |
| `src/lib/components/ForkIndicator.svelte` | **NEW** | Branch indicator |
| `src/lib/components/TrashPanel.svelte` | **NEW** | Trash browser |
| `src/lib/components/GhostBlock.svelte` | **NEW** | Ghost block display |

---

## Implementation Steps

### Step 1: Memory Lifecycle + Recall (~12k context)

1. Define lifecycle states and transition rules
2. Implement archive/offload behavior with metadata retention
3. Implement recall by topic, id, and keyword
4. Implement manifest generation from ground truth
5. Unit tests for transitions and recall

### Step 2: Hard Checkpoints (~10k context)

1. Define HardCheckpoint struct
2. Implement save (serialize full state)
3. Implement restore (deserialize and replace)
4. Add checkpoint storage (local filesystem)
5. Unit tests for save/restore

### Step 3: Soft Checkpoints (~10k context)

1. Define SoftCheckpoint struct
2. Implement summary generation (LLM or rule-based)
3. Extract files referenced, decisions made
4. Add injection into primacy zone
5. Unit tests for generation

### Step 4: Auto-Checkpointing (~7k context)

1. Define trigger events
2. Implement pattern detection (TODO, test, topic shift)
3. Add configuration system
4. Create auto-checkpoint indicator
5. Unit tests for trigger detection

### Step 5: Forking (~9k context)

1. Define Fork struct and ForkManager
2. Implement branch creation
3. Implement branch switching
4. Add comparison view
5. Implement merge logic
6. Unit tests for fork operations

### Step 6: Ghost & Trash (~7k context)

1. Implement ghost block generation
2. Implement trash storage
3. Add restore logic
4. Create TrashPanel UI
5. Add retention/purge policies
6. Unit tests for trash operations

---

## Test Coverage

### Unit Tests (~36 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/engine/memory.rs` | 6 | Lifecycle transitions |
| `src-tauri/src/engine/manifest.rs` | 4 | Manifest generation |
| `src-tauri/src/engine/recall.rs` | 4 | Recall behavior |
| `src-tauri/src/engine/checkpoint/hard.rs` | 6 | Save/restore |
| `src-tauri/src/engine/checkpoint/soft.rs` | 6 | Summary generation |
| `src-tauri/src/engine/checkpoint/auto.rs` | 4 | Trigger detection |
| `src-tauri/src/engine/fork.rs` | 8 | Fork operations |
| `src-tauri/src/engine/trash.rs` | 6 | Trash operations |

### Integration Tests (~8 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_memory_lifecycle.rs` | 2 | Archive/recall/manifest flow |
| `tests/integration/test_checkpoint_flow.rs` | 3 | Full checkpoint cycle |
| `tests/integration/test_fork_flow.rs` | 3 | Fork → edit → merge |

### Manual Tests (7 tests)

| Test | Description |
|------|-------------|
| `test_archive_recall` | Archive blocks then recall by topic/query |
| `test_hard_restore` | Save checkpoint, make changes, restore, verify |
| `test_soft_inject` | Create soft checkpoint, new session, inject |
| `test_auto_checkpoint` | Complete task, verify auto-checkpoint |
| `test_fork_compare` | Fork, make different changes, compare |
| `test_ghost_display` | Remove block, verify ghost appears |
| `test_trash_restore` | Remove block, restore from trash |

---

## Success Criteria

- [ ] Lifecycle transitions (hot/warm/cold/archived) are deterministic and reversible
- [ ] Manifest reflects ground truth state each turn
- [ ] Recall restores relevant blocks without data loss
- [ ] Hard checkpoints save and restore exact state
- [ ] Soft checkpoints generate useful summaries
- [ ] Auto-checkpointing triggers at meaningful boundaries
- [ ] Forking creates independent branches
- [ ] Fork comparison works side-by-side
- [ ] Ghost blocks appear when blocks removed
- [ ] Trash stores removed blocks with configurable retention
- [ ] Restore from trash works to original position
- [ ] `make check` passes
- [ ] 36+ unit tests passing
- [ ] 8+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::engine::checkpoint::{HardCheckpoint, SoftCheckpoint, AutoCheckpoint};
use crate::engine::fork::{Fork, ForkManager};
use crate::engine::ghost::generate_ghost;
use crate::engine::trash::{Trash, TrashEntry};
use crate::engine::memory::{MemoryState, MemoryTransition};
use crate::engine::manifest::ContextManifest;
use crate::engine::recall::RecallEngine;
```
