# Phase 11: System Prompts, A/B Testing & Git

**Status**: PENDING
**Goal**: Smart system prompt management, context A/B testing, git integration, and adaptive learning
**Prerequisites**: Phase 10 complete
**Estimated Scope**: ~55k context

---

## Context from Phase 10

Phase 10 delivers:
- TODO list integration with task parsing
- Task completion hooks (pause → compress → stage → resume)
- Predictive pre-staging for upcoming tasks
- Pause/swap system for context surgery
- Historical pattern learning

**Key imports:**
```rust
use crate::tasks::{TaskTracker, TrackedTask, TaskHook};
use crate::tasks::prefetch::PreFetcher;
use crate::proxy::pause::PauseController;
```

**Integration point:** Phase 11 adds experimentation and development workflow integration.

---

## Problem Statement

1. **No system prompt tooling** — System prompts are just blocks, no special handling
2. **No compression validation** — Can't verify compressed content preserves quality
3. **No git awareness** — Context not tied to code state
4. **No learning** — Don't adapt to user behavior over time
5. **Versioning lacks advanced workflows** — Need richer diff UX and correction-pattern intelligence

---

## Deliverables

### 1. Smart System Prompt Management

Dedicated tooling for system prompts:
- **Modular composition**: Build from pieces (base + project + task)
- **Version history**: Track changes over time
- **Effectiveness tracking**: Correlate prompt versions with session outcomes
- **Auto-composition**: Based on project + task, compose optimal prompt
- **A/B testing**: Try variants, compare results

```rust
pub struct SystemPromptModule {
    pub id: String,
    pub name: String,
    pub content: String,
    pub category: ModuleCategory,  // base | project | task | style
    pub tokens: usize,
}

pub struct ComposedPrompt {
    pub modules: Vec<SystemPromptModule>,
    pub total_tokens: usize,
    pub version: String,
}
```

### 2. Context A/B Testing

Empirically validate context decisions:
- Fork context at a point
- Send same prompt through both branches
- Compare responses side-by-side
- Track which produced better outcomes

Use cases:
- "Did compression cause the model to miss something?"
- "Is this system prompt variant better?"
- Automated mode: test before applying compression batch

### 3. Git Integration

Tie context to codebase:
- **Auto-checkpoint on commit**: Tag checkpoint with commit hash
- **"Show context from commit X"**: Restore exactly what AI had
- **`.aperture/` directory**: Store compressed snapshots in repo
- **Branch-aware presets**: Switching branches can switch presets
- **Pre-commit hooks**: "Warning: health score 52, 4 orphaned blocks"

### 4. Adaptive Learning

Learn from user behavior:
- Track patterns: "You consistently remove tool_result after 6 turns"
- Surface suggestions: "Create auto-rule? Done this in 14/20 sessions"
- Track compression preferences: "Compress file reads but never user messages"
- Track staging preferences: "Always load architecture doc on backend work"

Accept or reject suggestions — it learns from that too.

### 5. Advanced Block Versioning

Extend Phase 2 versioning:
- Version history for hot-patched blocks
- See diff between versions
- Undo specific edits
- Track correction patterns: "You've corrected path misunderstandings 8 times"

Enables: "Add a project structure doc to staging?"

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/system_prompt/mod.rs` | **NEW** | System prompt module |
| `src-tauri/src/system_prompt/composer.rs` | **NEW** | Prompt composition |
| `src-tauri/src/system_prompt/versions.rs` | **NEW** | Version tracking |
| `src-tauri/src/ab_test/mod.rs` | **NEW** | A/B testing |
| `src-tauri/src/git/mod.rs` | **NEW** | Git integration |
| `src-tauri/src/git/checkpoint.rs` | **NEW** | Commit-linked checkpoints |
| `src-tauri/src/git/hooks.rs` | **NEW** | Pre-commit hooks |
| `src-tauri/src/learning/mod.rs` | **NEW** | Adaptive learning |
| `src-tauri/src/learning/patterns.rs` | **NEW** | Pattern detection |
| `src-tauri/src/learning/suggestions.rs` | **NEW** | Suggestion generation |
| `src-tauri/src/engine/versioning.rs` | Modify | Advanced versioning + diff metadata |
| `src/lib/components/PromptComposer.svelte` | **NEW** | Prompt builder UI |
| `src/lib/components/ABComparison.svelte` | **NEW** | A/B comparison UI |
| `src/lib/components/LearningSuggestions.svelte` | **NEW** | Suggestions UI |

---

## Implementation Steps

### Step 1: System Prompt Management (~12k context)

1. Define module system
2. Implement composition logic
3. Add version tracking
4. Create composer UI
5. Add effectiveness metrics
6. Unit tests for composition

### Step 2: A/B Testing (~10k context)

1. Define A/B test structure
2. Implement fork + test flow
3. Create comparison UI
4. Add outcome tracking
5. Unit tests for A/B flow

### Step 3: Git Integration (~12k context)

1. Implement git operations (commit detection, hash extraction)
2. Create commit-linked checkpoints
3. Implement `.aperture/` storage
4. Add branch detection
5. Create pre-commit hook
6. Unit tests for git ops

### Step 4: Adaptive Learning (~12k context)

1. Define pattern types
2. Implement pattern detection
3. Create suggestion generator
4. Add feedback loop (accept/reject tracking)
5. Create suggestions UI
6. Unit tests for learning

### Step 5: Block Versioning (~9k context)

1. Define version structure
2. Implement version tracking
3. Add diff generation
4. Create version history UI
5. Implement undo
6. Unit tests for versioning

---

## Test Coverage

### Unit Tests (~35 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/system_prompt/composer.rs` | 8 | Composition logic |
| `src-tauri/src/ab_test/mod.rs` | 6 | A/B test flow |
| `src-tauri/src/git/mod.rs` | 8 | Git operations |
| `src-tauri/src/learning/patterns.rs` | 7 | Pattern detection |
| `src-tauri/src/engine/versioning.rs` | 6 | Version tracking |

### Integration Tests (~8 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_system_prompt.rs` | 3 | Composition flow |
| `tests/integration/test_git.rs` | 3 | Git checkpoint flow |
| `tests/integration/test_learning.rs` | 2 | Suggestion flow |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_prompt_compose` | Build prompt from modules, verify |
| `test_ab_comparison` | Fork, send prompt, compare responses |
| `test_git_checkpoint` | Commit code, verify checkpoint tagged |
| `test_branch_preset` | Switch branch, verify preset change |
| `test_learning_suggestion` | Repeat behavior, see suggestion |
| `test_block_undo` | Edit block, undo, verify restored |

---

## Success Criteria

- [ ] System prompts compose from modules
- [ ] Prompt versions tracked over time
- [ ] A/B testing forks and compares
- [ ] Git commits create tagged checkpoints
- [ ] Branch switching can switch presets
- [ ] Pre-commit hook warns on low health
- [ ] Adaptive learning surfaces suggestions
- [ ] Block versions tracked with undo
- [ ] `make check` passes
- [ ] 35+ unit tests passing
- [ ] 8+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::system_prompt::{PromptComposer, SystemPromptModule};
use crate::ab_test::{ABTest, ABResult};
use crate::git::{GitIntegration, CommitCheckpoint};
use crate::learning::{AdaptiveLearner, Suggestion};
use crate::engine::versioning::{BlockVersion, VersionHistory};
```
