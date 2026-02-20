# Phase 6: Staging, Presets & Templates

**Status**: PENDING
**Goal**: Pre-loaded context injection, named presets, templates, CLI flags, and project profiles
**Prerequisites**: Phase 5 complete
**Estimated Scope**: ~55k context

---

## Context from Phase 5

Phase 5 delivers:
- Hard checkpoints (exact state snapshots)
- Soft checkpoints (intelligent summaries for cross-session)
- Auto-checkpointing at meaningful boundaries
- Context forking with comparison and merge
- Ghost blocks for removed content
- Recently deleted trash with restore

**Key imports:**
```rust
use crate::engine::checkpoint::{HardCheckpoint, SoftCheckpoint};
use crate::engine::fork::{Fork, ForkManager};
use crate::engine::trash::Trash;
```

**Integration point:** Phase 6 adds configuration and workflow automation.

---

## Problem Statement

1. **No pre-loading** — Can't inject context before prompts
2. **No saved configurations** — Can't save pipeline + rules setups
3. **No structure templates** — Can't define session layouts
4. **No CLI integration** — Can't activate presets from command line
5. **No project awareness** — Config not tied to project directories

---

## Deliverables

### 1. Staging Area

Pre-loaded context blocks ready for injection:
- Drag files/docs into staging
- Each item has target zone (primacy/recency)
- Each item has injection condition (always/pattern-based)
- Priority ordering within zone
- Visual staging panel in UI

Use cases:
- Architecture doc always in primacy
- Coding conventions injected on session start
- Previous session summary injected on resume

```rust
pub struct StagedItem {
    pub id: String,
    pub content: String,
    pub target_zone: Zone,
    pub position: StagedPosition,  // start | end | after_system
    pub condition: InjectionCondition,
    pub priority: i32,
}

pub enum InjectionCondition {
    Always,
    OnSessionStart,
    WhenPattern(String),  // regex pattern in context
    WhenCluster(String),  // when topic cluster active
}
```

### 2. Named Presets

Saved configurations for different work modes:
- Pipeline stage toggles
- Auto-rule configurations
- Staging area contents
- Zone thresholds
- Model preferences

```yaml
preset: argus-backend
extends: python-project
pipeline:
  auto_condense: true
  condense_after_turns: 6
staging:
  - file: docs/architecture.md
    zone: primacy
    condition: always
  - file: .context/conventions.md
    zone: primacy
    condition: on_session_start
rules:
  - condense-tool-results-after-6
  - pin-errors-to-recency
```

Nested presets: `argus-backend` extends `argus-base` extends `python-project`

### 3. Context Templates

Define session **structure** (what types go where):
```yaml
template: debugging
structure:
  primacy:
    - system_prompt
    - error_context (pinned)
    - relevant_file_contents
  middle:
    - exploration_history
    - stale_tool_outputs
  recency:
    - recent_conversation (last 5 turns)
    - latest_tool_results
constraints:
  max_file_blocks: 3
  auto_condense_middle_after: 8 turns
  pin_errors: true
```

Templates define layout, presets define content. They compose.

### 4. CLI Integration

Activate presets from command line:
```bash
# Load preset before prompt
claude --aperture-preset argus-backend "add rate limiting"

# Or via aperture command
aperture load-preset argus-backend
aperture exec "compress the safety cluster"
```

Environment variable: `APERTURE_PRESET=argus-backend`

### 5. Project Profiles

Project-level config in `.aperture.yml`:
```yaml
project: argus
default_preset: argus-backend
default_template: backend-dev
provider: claude-code
token_budget: 200000

watch_paths:
  - src/
  - tests/

auto_rules:
  - condense-tool-results-after-6
  - deduplicate-file-reads

topic_clusters:
  - helios (market data)
  - athena (strategy)
```

Auto-loads when `cd` into project and launch Aperture.

### 6. Quick-Switch UI

Fast preset/template switching:
- Dropdown in header
- Keyboard shortcut (P for presets, T for templates)
- Recent presets list
- Preset comparison view

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/engine/staging.rs` | **NEW** | Staging area |
| `src-tauri/src/engine/preset.rs` | **NEW** | Preset system |
| `src-tauri/src/engine/template.rs` | **NEW** | Template system |
| `src-tauri/src/engine/profile.rs` | **NEW** | Project profiles |
| `src-tauri/src/cli.rs` | **NEW** | CLI commands |
| `src/lib/components/StagingPanel.svelte` | **NEW** | Staging UI |
| `src/lib/components/PresetPicker.svelte` | **NEW** | Preset dropdown |
| `src/lib/components/TemplatePicker.svelte` | **NEW** | Template dropdown |

---

## Implementation Steps

### Step 1: Staging Area (~12k context)

1. Define StagedItem struct
2. Implement staging storage
3. Implement injection logic
4. Create StagingPanel UI
5. Add drag-drop into staging
6. Unit tests for injection

### Step 2: Preset System (~12k context)

1. Define Preset struct with YAML serialization
2. Implement preset loading/saving
3. Implement preset inheritance (extends)
4. Add preset application to engine
5. Unit tests for presets

### Step 3: Template System (~10k context)

1. Define Template struct
2. Implement template parsing
3. Implement structure enforcement
4. Add template + preset composition
5. Unit tests for templates

### Step 4: CLI Integration (~10k context)

1. Create CLI argument parser
2. Implement `aperture load-preset`
3. Implement `aperture exec`
4. Add environment variable support
5. Integration tests for CLI

### Step 5: Project Profiles (~8k context)

1. Implement `.aperture.yml` parsing
2. Add auto-detection on directory change
3. Implement profile application
4. Create profile editor UI
5. Unit tests for profiles

### Step 6: Quick-Switch UI (~3k context)

1. Create PresetPicker dropdown
2. Create TemplatePicker dropdown
3. Add keyboard shortcuts
4. Add recent list

---

## Test Coverage

### Unit Tests (~30 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/engine/staging.rs` | 8 | Injection logic |
| `src-tauri/src/engine/preset.rs` | 8 | Preset loading, inheritance |
| `src-tauri/src/engine/template.rs` | 6 | Template parsing, enforcement |
| `src-tauri/src/engine/profile.rs` | 4 | Profile loading |
| `src-tauri/src/cli.rs` | 4 | CLI parsing |

### Integration Tests (~8 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_preset_flow.rs` | 4 | Load → apply → verify |
| `tests/integration/test_profile.rs` | 4 | Auto-load on directory |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_staging_injection` | Stage doc, new session, verify injected |
| `test_preset_switch` | Switch presets, verify config changes |
| `test_template_enforcement` | Apply template, verify structure |
| `test_cli_preset` | `claude --aperture-preset`, verify loaded |
| `test_project_profile` | cd into project, verify auto-load |
| `test_preset_inheritance` | Nested preset, verify extension |

---

## Success Criteria

- [ ] Staging area stores and injects pre-loaded content
- [ ] Injection conditions work (always, pattern, cluster)
- [ ] Presets save and load all configuration
- [ ] Preset inheritance works
- [ ] Templates define and enforce structure
- [ ] CLI flags activate presets
- [ ] Project profiles auto-load on directory
- [ ] Quick-switch UI works with keyboard
- [ ] `make check` passes
- [ ] 30+ unit tests passing
- [ ] 8+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::engine::staging::{StagingArea, StagedItem, InjectionCondition};
use crate::engine::preset::{Preset, PresetManager};
use crate::engine::template::{Template, TemplateManager};
use crate::engine::profile::ProjectProfile;
```
