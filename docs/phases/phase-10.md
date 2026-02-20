# Phase 10: Task Integration & Pre-Fetching

**Status**: PENDING
**Goal**: TODO list integration, task completion hooks, and predictive context pre-staging
**Prerequisites**: Phase 9 complete
**Estimated Scope**: ~50k context

---

## Context from Phase 9

Phase 9 delivers:
- Token analytics dashboard with cost breakdown
- Token cost forecasting ("limit in ~N turns")
- Session timeline view with zoom/pan
- Context replay (scrub through history)
- Context health score (0-100)
- Warning system with configurable thresholds

**Key imports:**
```rust
use crate::analytics::{CostAnalytics, Forecast, HealthScore};
use crate::analytics::warnings::{Warning, WarningConfig};
```

**Integration point:** Phase 10 adds task-aware context management.

---

## Problem Statement

1. **No task awareness** — Aperture doesn't know what the agent is working on
2. **No context transitions** — Task completion doesn't trigger context cleanup
3. **No predictive loading** — Don't pre-stage context for upcoming tasks
4. **Pause exists but not task-aware** — No transactional pause/swap flow tied to task hooks
5. **No historical patterns** — Don't learn from past task→file associations

---

## Deliverables

### 1. TODO List Integration

Read and monitor agent's task list:
- Detect TODO patterns in context (TodoWrite, `- [ ]` lists)
- Parse task structure (title, status, dependencies)
- Track task completion events
- Display current task in Aperture UI

```rust
pub struct TrackedTask {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,  // pending | in_progress | complete
    pub context_blocks: Vec<String>,  // blocks related to this task
    pub files_touched: Vec<PathBuf>,
}
```

### 2. Task Completion Hooks

Automated actions on task completion:
```
Agent completes task 3 → marks it off
         ↓
Hook fires → Aperture pauses agent
         ↓
Compresses task 3 context (keep results, compress exploration)
         ↓
Pre-loads context for task 4
         ↓
Resumes agent with clean, focused context
```

Configurable per-project:
- Auto-checkpoint on task completion
- Auto-compress completed task context
- Auto-stage next task context
- Compression level for completed tasks

### 3. Predictive Pre-Staging

Based on task list, pre-compute next task's context:
- Analyze task description for file/doc mentions
- Pre-load likely relevant files to staging
- Pre-generate compression levels
- Historical patterns: "when working on auth, you always reference middleware config within 3 turns"

### 4. Transactional Pause/Swap

Extend existing pause/hold capability into a transactional flow:
- "Pause" holds next outbound request
- Swap context while paused
- "Resume" continues with modified context
- Agent never knows it happened

Use cases:
- Task transition cleanup
- Emergency context reduction
- Manual context injection

### 5. Historical Pattern Learning

Learn from past sessions:
- Track task → files accessed patterns
- Track task → topic clusters patterns
- Build predictive model over time
- Surface suggestions: "Based on history, you'll need auth.rs"

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/tasks/mod.rs` | **NEW** | Task integration module |
| `src-tauri/src/tasks/parser.rs` | **NEW** | Task list parsing |
| `src-tauri/src/tasks/hooks.rs` | **NEW** | Completion hooks |
| `src-tauri/src/tasks/prefetch.rs` | **NEW** | Pre-staging logic |
| `src-tauri/src/tasks/patterns.rs` | **NEW** | Historical patterns |
| `src-tauri/src/proxy/pause.rs` | Modify | Transactional pause/swap orchestration |
| `src/lib/components/TaskPanel.svelte` | **NEW** | Task display |
| `src/lib/components/PauseControls.svelte` | **NEW** | Pause UI |

---

## Implementation Steps

### Step 1: Task Parsing (~10k context)

1. Define task detection patterns
2. Implement TODO list parser
3. Track task status changes
4. Create task display UI
5. Unit tests for parsing

### Step 2: Completion Hooks (~12k context)

1. Define hook configuration
2. Implement hook trigger system
3. Create compression action
4. Create staging action
5. Integrate with checkpoint system
6. Unit tests for hooks

### Step 3: Pre-Staging (~10k context)

1. Analyze task descriptions for files
2. Implement pre-load logic
3. Add pre-compression
4. Integrate with staging area
5. Unit tests for pre-staging

### Step 4: Transactional Pause/Swap (~10k context)

1. Reuse request hold system from Phase 1
2. Create swap logic
3. Build pause UI
4. Add resume flow
5. Unit tests for pause/swap

### Step 5: Pattern Learning (~8k context)

1. Define pattern storage
2. Implement pattern extraction
3. Create suggestion generator
4. Build suggestion UI
5. Unit tests for patterns

---

## Test Coverage

### Unit Tests (~30 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/tasks/parser.rs` | 8 | Task parsing |
| `src-tauri/src/tasks/hooks.rs` | 8 | Hook triggers |
| `src-tauri/src/tasks/prefetch.rs` | 6 | Pre-staging |
| `src-tauri/src/tasks/patterns.rs` | 4 | Pattern detection |
| `src-tauri/src/proxy/pause.rs` | 4 | Pause/swap |

### Integration Tests (~6 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_task_hooks.rs` | 3 | Hook execution |
| `tests/integration/test_pause.rs` | 3 | Pause/swap flow |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_task_detection` | Agent uses TodoWrite, verify detected |
| `test_completion_hook` | Complete task, verify compression |
| `test_pre_staging` | Task 4 has file mention, verify pre-loaded |
| `test_pause_swap` | Pause, modify context, resume |
| `test_pattern_suggestion` | Work on auth twice, verify suggestion |
| `test_hook_config` | Configure hook, verify behavior |

---

## Success Criteria

- [ ] Task list parsed from context
- [ ] Task completion detected automatically
- [ ] Hooks fire on task completion
- [ ] Context compressed for completed tasks
- [ ] Next task context pre-staged
- [ ] Pause/hold integrates with task hooks without deadlocks
- [ ] Swap modifies context while paused
- [ ] Historical patterns tracked and surfaced
- [ ] `make check` passes
- [ ] 30+ unit tests passing
- [ ] 6+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::tasks::{TaskTracker, TrackedTask, TaskHook};
use crate::tasks::prefetch::PreFetcher;
use crate::tasks::patterns::PatternLearner;
use crate::proxy::pause::{PauseController, SwapContext};
```
