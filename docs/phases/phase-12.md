# Phase 12: Plugin System & Ecosystem

**Status**: PENDING
**Goal**: Plugin architecture, community features, and multi-agent context sharing
**Prerequisites**: Phase 11 complete
**Estimated Scope**: ~50k context

---

## Context from Phase 11

Phase 11 delivers:
- Smart system prompt management with modular composition
- Context A/B testing for quality validation
- Git integration (commit checkpoints, branch-aware presets)
- Adaptive learning with behavioral suggestions
- Block versioning with edit history and undo

**Key imports:**
```rust
use crate::system_prompt::PromptComposer;
use crate::ab_test::ABTest;
use crate::git::GitIntegration;
use crate::learning::AdaptiveLearner;
use crate::engine::versioning::VersionHistory;
```

**Integration point:** Phase 12 opens the platform for extension and collaboration.

---

## Problem Statement

1. **No extensibility** — Can't add custom pipeline stages
2. **No community sharing** — Can't share presets/rules/templates
3. **No multi-agent support** — Can't coordinate context across agents
4. **No plugin marketplace** — Hard to discover extensions
5. **No API for developers** — Can't build on top of Aperture

---

## Deliverables

### 1. Plugin System

Custom pipeline stages that users can write and share:

```rust
pub trait PipelinePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn stage(&self) -> PipelinePosition;  // before_parse | after_zone | etc.
    fn process(&self, blocks: &mut Vec<Block>, config: &PluginConfig) -> Result<()>;
    fn ui_panel(&self) -> Option<PluginUI>;  // optional config UI
}
```

Plugin capabilities:
- Modify blocks during pipeline
- Add custom UI panels
- Access engine state
- Emit events
- Store plugin-specific data

### 2. Plugin Distribution

Easy plugin installation:
- Plugins as Rust crates or WASM modules
- Community plugin registry (like crates.io)
- CLI install: `aperture plugin install auto-test-staging`
- Automatic updates
- Sandboxed execution for safety

### 3. Example Plugins

Ship with useful examples:
- **auto-test-staging**: Watch source files, auto-stage corresponding tests
- **cost-optimizer**: Route to cheaper models when context is boilerplate
- **team-conventions**: Enforce project-specific rules
- **metrics-collector**: Push stats to Grafana/Prometheus
- **voice-notes**: Dictate annotations

### 4. Community Library

Share configurations:
- Preset sharing/export
- Rule library
- Template library
- Plugin ratings and reviews
- User profiles

### 5. Developer API

Build on Aperture:
- Local HTTP API for integrations
- WebSocket API for real-time data
- CLI scripting interface
- Documentation and examples

```bash
# Query current context
curl http://localhost:5401/api/v1/blocks

# Apply compression
curl -X POST http://localhost:5401/api/v1/blocks/compress \
  -d '{"ids": ["block-1", "block-2"], "level": "summarized"}'
```

### 6. Multi-Agent Context Sharing (Deferred Complexity)

Bridge context between multiple agents:
- Backend agent (Claude Code) + frontend agent (Codex)
- Key findings from one inject into other's staging
- Configure which clusters propagate
- Conflict resolution

**Note:** This is complex. Implementation is best-effort in Phase 12, may extend to future phases.

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/plugin/mod.rs` | **NEW** | Plugin system |
| `src-tauri/src/plugin/loader.rs` | **NEW** | Plugin loading |
| `src-tauri/src/plugin/registry.rs` | **NEW** | Plugin registry |
| `src-tauri/src/plugin/sandbox.rs` | **NEW** | Sandboxed execution |
| `src-tauri/src/plugin/wasm.rs` | **NEW** | WASM plugin support |
| `src-tauri/src/api/mod.rs` | **NEW** | HTTP API |
| `src-tauri/src/api/routes.rs` | **NEW** | API routes |
| `src-tauri/src/community/mod.rs` | **NEW** | Community features |
| `src-tauri/src/multiagent/mod.rs` | **NEW** | Multi-agent (experimental) |
| `plugins/auto-test-staging/` | **NEW** | Example plugin |
| `plugins/cost-optimizer/` | **NEW** | Example plugin |
| `src/lib/components/PluginPanel.svelte` | **NEW** | Plugin management UI |
| `src/lib/components/CommunityBrowser.svelte` | **NEW** | Community library UI |

---

## Implementation Steps

### Step 1: Plugin Architecture (~15k context)

1. Define PipelinePlugin trait
2. Implement plugin loading (native + WASM)
3. Create plugin lifecycle management
4. Add sandboxing for safety
5. Integrate with pipeline
6. Unit tests for plugin system

### Step 2: Plugin CLI (~8k context)

1. Implement `aperture plugin list`
2. Implement `aperture plugin install`
3. Implement `aperture plugin remove`
4. Add update checking
5. Unit tests for CLI

### Step 3: Developer API (~12k context)

1. Create HTTP API server
2. Define API routes
3. Implement WebSocket events
4. Add authentication (local-only by default)
5. Write API documentation
6. Unit tests for API

### Step 4: Example Plugins (~8k context)

1. Create auto-test-staging plugin
2. Create cost-optimizer plugin
3. Create metrics-collector plugin
4. Document plugin development
5. Integration tests

### Step 5: Community Features (~7k context)

1. Define sharing format
2. Create export/import logic
3. Build community browser UI
4. Add basic discovery features
5. Unit tests for community

---

## Test Coverage

### Unit Tests (~30 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/plugin/loader.rs` | 8 | Plugin loading |
| `src-tauri/src/plugin/sandbox.rs` | 6 | Sandboxing |
| `src-tauri/src/api/routes.rs` | 10 | API endpoints |
| `src-tauri/src/community/mod.rs` | 6 | Share/import |

### Integration Tests (~6 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_plugin.rs` | 3 | Plugin lifecycle |
| `tests/integration/test_api.rs` | 3 | API endpoints |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_plugin_install` | Install plugin, verify loaded |
| `test_plugin_execute` | Plugin runs during pipeline |
| `test_api_query` | Query blocks via HTTP API |
| `test_api_modify` | Modify context via API |
| `test_export_preset` | Export preset, verify format |
| `test_import_preset` | Import shared preset, verify applies |

---

## Success Criteria

- [ ] Plugin trait defined and documented
- [ ] Plugins load and execute in pipeline
- [ ] WASM plugins supported
- [ ] Plugin sandboxing prevents unsafe operations
- [ ] CLI plugin management works
- [ ] HTTP API serves block data
- [ ] WebSocket pushes real-time events
- [ ] Example plugins functional
- [ ] Preset export/import works
- [ ] `make check` passes
- [ ] 30+ unit tests passing
- [ ] 6+ integration tests passing
- [ ] All manual tests documented and passing

---

## Post-Phase 12: Future Directions

### Multi-Agent (v13+)
Full multi-agent context sharing:
- Cross-agent knowledge propagation
- Conflict resolution
- Agent-specific views

### Mobile Companion (v14+)
- Mobile app for monitoring
- Push notifications for warnings
- Remote context viewing

### Cloud Sync (v15+, Optional)
- Encrypted cloud backup
- Cross-device sync
- Team sharing (enterprise)

### Voice Interface (v16+)
- Voice commands
- Voice annotations
- Hands-free operation

---

## Final Notes

Phase 12 marks the completion of Aperture's core feature set. The product is now:

1. **Fully functional** — All core features implemented
2. **Extensible** — Plugin system for custom functionality
3. **Shareable** — Community library for configurations
4. **Programmable** — API for integrations
5. **Future-ready** — Architecture supports multi-agent and beyond

The foundation is complete. What comes next is polish, community building, and listening to users.
