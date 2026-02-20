# Phase 7: Cleaner Model Sidecar

**Status**: PENDING
**Goal**: Local model sidecar for background intelligence, tiered routing, and semantic enrichment of existing engine systems
**Prerequisites**: Phase 6 complete
**Estimated Scope**: ~55k context

---

## Context from Phase 6

Phase 6 delivers:
- Staging area with pre-loaded context injection
- Named presets with inheritance
- Context templates defining session structure
- CLI integration (`--aperture-preset`)
- Project profiles (`.aperture.yml`)
- Quick-switch UI for presets/templates

**Key imports:**
```rust
use crate::engine::staging::{StagingArea, StagedItem};
use crate::engine::preset::{Preset, PresetManager};
use crate::engine::template::{Template, TemplateManager};
```

**Integration point:** Phase 7 adds intelligence via local and API models.

**Dependency note from Phase 3:**
- Phase 3 introduces a compression-scoped backend interface/config path.
- Phase 7 generalizes that interface into a shared sidecar runtime for multiple task classes.

---

## Problem Statement

1. **No unified sidecar runtime** — Background model tasks are not centrally orchestrated
2. **No local model backend** — All higher-quality analysis requires external APIs
3. **No tiered task routing** — Can't send tasks to cost-appropriate model tiers
4. **No semantic dependency enrichment** — Deterministic graph exists but lacks semantic relationships
5. **No robust quality verification** — Can't reliably validate model-generated outputs

---

## Deliverables

### 1. Cleaner Model Sidecar

Background worker for LLM-powered tasks:
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

Never blocks proxy or UI. Results available on next request.

### 2. Model Backend Abstraction (Generalized from Phase 3)

Support multiple model providers:
- **Ollama** — Easy local model management
- **llama.cpp** — Direct inference (optional)
- **Anthropic API** — Haiku for mid-tier tasks
- **OpenAI API** — GPT-4 mini for mid-tier tasks
- **OpenRouter API** — Optional third-party model routing

```rust
pub trait ModelBackend: Send + Sync {
    async fn complete(&self, prompt: &str, config: &ModelConfig) -> Result<String>;
    async fn health_check(&self) -> Result<()>;
    fn name(&self) -> &str;
}
```

### 3. Tiered Model Selection

Route tasks to appropriate capability tier:

**Tier 1 — Routine** (background, high volume):
- Default: Local model (Qwen 2.5 3B, Phi-3 Mini)
- Tasks: Trimmed compression, topic classification, dedup detection

**Tier 2 — Significant** (fewer, higher stakes):
- Default: Mid-tier API (Haiku, GPT-4 mini)
- Tasks: Summarized compression, soft checkpoints, quality verification

**Tier 3 — Critical** (rare, maximum fidelity):
- Default: Strong model (Sonnet, GPT-4)
- Tasks: Major summaries, cross-session memory, complex reasoning chains

Fully configurable: any task → any model.

### 4. Task Queue System

Priority-based async processing:
- Tasks queued with priority level
- Batch processing for efficiency
- Progress tracking per task
- Cancellation support
- Retry with exponential backoff

### 5. Semantic Dependency Enrichment

Track relationships between blocks:
- **File reference chains** — Read file → later blocks use it
- **Conversation flow** — User message → assistant response
- **Tool chains** — tool_use → tool_result → reasoning
- **Information propagation** — Block B quotes from Block A

Extends Phase 2 deterministic dependency tracking with semantic edges.

Enables:
- "Removing this will orphan 3 blocks" warnings
- Cascade compression (propagate info to dependents)
- Smart removal ordering
- Impact visualization

### 6. Quality Verification

Validate LLM outputs:
- Score compression quality (does it preserve key info?)
- Flag bad compressions for review
- Track quality metrics over time
- Auto-retry with stronger model if quality fails

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/sidecar/mod.rs` | **NEW** | Sidecar module |
| `src-tauri/src/sidecar/queue.rs` | **NEW** | Task queue |
| `src-tauri/src/sidecar/backend.rs` | **NEW** | Model backend trait |
| `src-tauri/src/sidecar/ollama.rs` | **NEW** | Ollama integration |
| `src-tauri/src/sidecar/anthropic.rs` | **NEW** | Anthropic API |
| `src-tauri/src/sidecar/router.rs` | **NEW** | Task → model routing |
| `src-tauri/src/sidecar/quality.rs` | **NEW** | Quality verification |
| `src-tauri/src/engine/dependency.rs` | Modify | Add semantic dependency enrichment |
| `src/lib/components/SidecarStatus.svelte` | **NEW** | Status indicator |
| `src/lib/components/DependencyView.svelte` | **NEW** | Graph visualization |

---

## Implementation Steps

### Step 1: Model Backend (~12k context)

1. Define ModelBackend trait
2. Implement Ollama backend
3. Implement Anthropic backend
4. Add health checking
5. Unit tests with mocked backends

### Step 2: Task Queue (~12k context)

1. Define Task struct with priority
2. Implement priority queue
3. Add batch processing logic
4. Implement cancellation
5. Add retry logic
6. Unit tests for queue operations

### Step 3: Tiered Routing (~10k context)

1. Define task types and tiers
2. Implement routing configuration
3. Add fallback chains
4. Integrate with compression system
5. Unit tests for routing

### Step 4: Dependency Enrichment (~12k context)

1. Define dependency edge types
2. Extend deterministic graph with semantic edge extraction
3. Add orphan detection
4. Implement cascade analysis
5. Create visualization component
6. Unit tests for graph operations

### Step 5: Quality Verification (~9k context)

1. Define quality metrics
2. Implement scoring algorithm
3. Add flagging logic
4. Implement auto-retry
5. Unit tests for verification

---

## Test Coverage

### Unit Tests (~35 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/sidecar/queue.rs` | 8 | Queue operations |
| `src-tauri/src/sidecar/ollama.rs` | 4 | Ollama integration |
| `src-tauri/src/sidecar/router.rs` | 8 | Task routing |
| `src-tauri/src/sidecar/quality.rs` | 6 | Quality scoring |
| `src-tauri/src/engine/dependency.rs` | 9 | Graph operations |

### Integration Tests (~8 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_sidecar.rs` | 4 | End-to-end sidecar flow |
| `tests/integration/test_dependency.rs` | 4 | Deterministic + semantic dependency behavior |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_ollama_integration` | Local model processes compression |
| `test_tier_routing` | Different tasks go to different models |
| `test_queue_progress` | UI shows task progress |
| `test_dependency_warning` | Remove block, see orphan warning |
| `test_quality_flag` | Bad compression flagged for review |
| `test_cascade_compress` | Compress block, dependents updated |

---

## Success Criteria

- [ ] Sidecar runs as async background worker
- [ ] Ollama backend connects and processes tasks
- [ ] Anthropic backend works as fallback
- [ ] Task queue processes with priority ordering
- [ ] Tiered routing sends tasks to correct models
- [ ] Dependency graph includes deterministic + semantic relationships
- [ ] Orphan warnings appear on block removal
- [ ] Quality verification flags bad compressions
- [ ] `make check` passes
- [ ] 35+ unit tests passing
- [ ] 8+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::sidecar::{Sidecar, Task, TaskPriority, ModelTier};
use crate::sidecar::queue::TaskQueue;
use crate::sidecar::router::TaskRouter;
use crate::engine::dependency::{DependencyGraph, DependencyEdge};
```
