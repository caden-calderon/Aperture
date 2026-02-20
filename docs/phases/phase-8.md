# Phase 8: Search & Natural Language Commands

**Status**: PENDING
**Goal**: Full-text and semantic search, NL commands, and block annotations
**Prerequisites**: Phase 7 complete
**Estimated Scope**: ~50k context

---

## Context from Phase 7

Phase 7 delivers:
- Cleaner model sidecar for background tasks
- Model backend abstraction (Ollama, Anthropic, OpenAI)
- Tiered model selection (local → mid-tier → strong)
- Priority-based task queue
- Block dependency graph with orphan warnings
- Quality verification for LLM outputs

**Key imports:**
```rust
use crate::sidecar::{Sidecar, Task, TaskPriority};
use crate::sidecar::router::TaskRouter;
use crate::engine::dependency::DependencyGraph;
```

**Integration point:** Phase 8 adds searchability and natural language interaction.

---

## Problem Statement

1. **No search** — Can't find content in current or historical context
2. **No semantic understanding** — Keyword-only, no "related to X"
3. **No NL interface** — Must click through UI for every operation
4. **No annotations** — Can't tag blocks with notes
5. **No block selection by content** — Can't "select everything with auth"

---

## Deliverables

### 1. Full-Text Search

Search across context:
- Current session search (instant)
- Historical session search (indexed)
- Filter by block type, zone, date range
- Regex support
- Highlight matches in results

### 2. Semantic Search

LLM-assisted search via cleaner model:
- "Find everything related to database connection pooling"
- Works even without exact keywords
- Powered by local model for privacy

**Embeddings (opt-in, off by default):**
- Generate embeddings for blocks via local model
- Store in SQLite (BLOB column)
- Enables faster similarity search for large historical datasets
- User enables in settings: "Generate embeddings for semantic search"
- Configurable: which model, when to generate (on block create, background batch, manual)

**Philosophy:** Everything is customizable. Users who want fast semantic search can enable embeddings. Users who want minimal resource usage can rely on LLM-assisted search only.

### 3. Keyword-Based Selection

Search → select workflow:
- Type search term
- All matching blocks highlighted
- "Select all matches" button
- "Compress non-matches" button
- Enables bulk operations on search results

### 4. Natural Language Commands

Type commands in command palette:
```
> compress everything from before the auth discussion
> pin the last three tool outputs
> remove all file reads older than 20 turns
> save a checkpoint called pre-refactor
> show me what changed since the last checkpoint
> expand all safety-related blocks
> find everything about WebSocket and select it
```

Cleaner model parses intent → maps to engine operations → shows preview → executes on confirmation.

Also works from terminal:
```bash
aperture exec "compress the safety cluster to summarized"
```

### 5. Block Annotations

Attach persistent notes to blocks:
- Select block → `N` key → type note
- Annotations survive compression
- Searchable (become personal index)
- Relevance boost for annotated blocks
- Visual badge on annotated blocks

### 6. Search History

Track and reuse searches:
- Recent searches list
- Saved searches
- Search analytics ("you search for 'auth' a lot")

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/search/mod.rs` | **NEW** | Search module |
| `src-tauri/src/search/fulltext.rs` | **NEW** | Full-text search |
| `src-tauri/src/search/semantic.rs` | **NEW** | Semantic search |
| `src-tauri/src/search/index.rs` | **NEW** | Search index |
| `src-tauri/src/nlp/mod.rs` | **NEW** | NLP module |
| `src-tauri/src/nlp/parser.rs` | **NEW** | NL command parser |
| `src-tauri/src/nlp/commands.rs` | **NEW** | Command definitions |
| `src-tauri/src/engine/annotation.rs` | **NEW** | Annotation system |
| `src/lib/components/controls/SearchBar.svelte` | Modify | Existing search UI upgraded for engine-backed search |
| `src/lib/components/SearchResults.svelte` | **NEW** | Results display |
| `src/lib/components/controls/CommandPalette.svelte` | Modify | Add NL commands |
| `src/lib/components/AnnotationBadge.svelte` | **NEW** | Annotation indicator |

---

## Implementation Steps

### Step 1: Full-Text Search (~12k context)

1. Implement in-memory search for current session
2. Create search index for historical sessions
3. Add filter options (type, zone, date)
4. Implement regex support
5. Add match highlighting
6. Unit tests for search

### Step 2: Semantic Search (~10k context)

1. Create semantic search prompts
2. Integrate with cleaner sidecar
3. Implement embedding-based search (optional)
4. Add relevance ranking
5. Unit tests with mocked LLM

### Step 3: Selection Workflow (~8k context)

1. Implement search → highlight flow
2. Add "select all matches" action
3. Add "compress non-matches" action
4. Integrate with selection store
5. Unit tests for selection

### Step 4: NL Commands (~12k context)

1. Define command vocabulary
2. Create NL → operation parser
3. Implement preview generation
4. Add confirmation flow
5. Integrate with CLI
6. Unit tests for parsing

### Step 5: Annotations (~8k context)

1. Define Annotation struct
2. Add annotation CRUD operations
3. Implement annotation search
4. Create annotation UI
5. Add relevance boost logic
6. Unit tests for annotations

---

## Test Coverage

### Unit Tests (~30 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/search/fulltext.rs` | 8 | Search accuracy |
| `src-tauri/src/search/semantic.rs` | 4 | Semantic matching |
| `src-tauri/src/search/index.rs` | 6 | Index operations |
| `src-tauri/src/nlp/parser.rs` | 8 | Command parsing |
| `src-tauri/src/engine/annotation.rs` | 4 | Annotation ops |

### Integration Tests (~6 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_search.rs` | 3 | End-to-end search |
| `tests/integration/test_nlp.rs` | 3 | NL command flow |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_fulltext_search` | Search keyword, verify matches |
| `test_semantic_search` | Search concept, verify related blocks |
| `test_select_matches` | Search, select all, verify selection |
| `test_nl_compress` | "compress old tool outputs", verify |
| `test_annotation_create` | Add note, verify appears |
| `test_cli_exec` | `aperture exec`, verify operation |

---

## Success Criteria

- [ ] Full-text search returns accurate results
- [ ] Historical search works across sessions
- [ ] Semantic search finds related content
- [ ] Keyword → select all workflow works
- [ ] NL commands parse and execute correctly
- [ ] Command preview shows before execution
- [ ] Annotations persist and are searchable
- [ ] CLI `aperture exec` works
- [ ] `make check` passes
- [ ] 30+ unit tests passing
- [ ] 6+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::search::{SearchEngine, SearchQuery, SearchResult};
use crate::search::semantic::semantic_search;
use crate::nlp::{parse_command, NLCommand};
use crate::engine::annotation::{Annotation, AnnotationManager};
```
