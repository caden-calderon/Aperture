# Phase 4: Heat Map & Topic Clustering

**Status**: PENDING
**Goal**: Usage heat tracking, attention analysis, topic clustering, and semantic deduplication
**Prerequisites**: Phase 3 complete
**Estimated Scope**: ~50k context

---

## Context from Phase 3

Phase 3 delivers:
- Four-level compression system (original → trimmed → summarized → minimal)
- Rule-based and LLM-based compression
- Async compression queue
- Compression slider UI with preview
- Preserve-keys system for critical content

**Key imports:**
```rust
use crate::engine::compression::{CompressionLevel, CompressionQueue};
```

**Integration point:** Phase 4 adds intelligence about block relevance and relationships.

---

## Problem Statement

1. **No heat tracking** — Don't know which blocks the model actually uses
2. **No position relevance** — Can't predict attention based on primacy/recency
3. **No topic grouping** — Blocks are individual, no semantic relationships
4. **No deduplication** — Same file read 3 times wastes tokens
5. **No visual feedback** — Heat and relevance not visible in UI

---

## Deliverables

### 1. Usage Heat Tracking

Track how much each block is referenced in responses:
- Analyze model responses for references to source blocks
- Match quoted content, file names, function names
- Track `reference_count` and `last_referenced_turn`
- Calculate `usage_heat` (0.0-1.0) based on recency-weighted references

### 2. Position Relevance Scoring

Predictive attention based on position:
- Primacy zone → high relevance (0.8-1.0)
- Recency zone → high relevance (0.7-0.9)
- Middle zone → lower relevance, decays with age
- Calculate `position_relevance` (0.0-1.0)

### 3. Heat Visualization

Visual encoding of heat in UI:
- **Dot density** = usage heat (dense = hot, sparse = cold)
- **Dot color** = position relevance (bright teal = high, dim gray = low)
- Smooth transitions (~500ms) on heat updates
- "Fading away" effect for cold blocks

The combination matrix:
- High heat + high relevance = **Critical** (keep expanded)
- High heat + low relevance = **Mispositioned** (suggest move)
- Low heat + high relevance = **Underperforming** (watch)
- Low heat + low relevance = **Dead weight** (compress/remove)

### 4. Topic Clustering

Lightweight, no-LLM clustering:
- Extract keywords from each block
- Calculate keyword overlap between blocks
- Cluster blocks with high overlap
- Assign topic labels (auto-generated or user-defined)

Enables:
- Group operations (compress all blocks in "auth" topic)
- Dynamic rebalancing on topic pivots
- Color-coded cluster badges in UI

### Dynamic Rebalancing (Context Breathing)

When the agent switches tasks (e.g., finishes "safety" work, starts "plugins"):

**Detection (both automatic and manual):**
- **Automatic:** Detect topic shift from user messages (keyword analysis, cluster change)
- **Manual:** User clicks "switch to topic X" or uses command palette
- Manual always overrides automatic detection

**Rebalancing flow:**
1. Pause outbound request using Phase 1 hold primitive (or trigger on task completion hook)
2. Identify old topic's blocks → compress to summarized/minimal
3. Identify new topic's blocks → expand to original/trimmed
4. Resume with rebalanced context

**Configuration:**
- Rebalancing aggressiveness (how much to compress old topic)
- Animation speed (instant vs. smooth transition)
- Auto-rebalance toggle (can disable for manual-only control)

### 5. Semantic Deduplication

Detect redundant content:
- Exact duplicates (same file read twice)
- Near duplicates (file read with minor changes)
- Configurable similarity threshold
- Auto-consolidation option (keep most recent, compress others)

### 6. Heat-Based Recommendations

Surface actionable insights:
- "5 blocks have zero heat for 10+ turns — remove?"
- "Block #7 is frequently referenced but buried in middle — pin to primacy?"
- "3 near-duplicate file reads — consolidate?"

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/engine/heat.rs` | **NEW** | Heat calculation |
| `src-tauri/src/engine/attention.rs` | **NEW** | Response analysis for references |
| `src-tauri/src/engine/clustering.rs` | **NEW** | Topic clustering |
| `src-tauri/src/engine/dedup.rs` | **NEW** | Deduplication detection |
| `src-tauri/src/engine/recommendations.rs` | **NEW** | Heat-based suggestions |
| `src/lib/components/HeatOverlay.svelte` | **NEW** | Canvas heat visualization |
| `src/lib/components/ClusterBadge.svelte` | **NEW** | Topic cluster badge |
| `src/lib/components/Recommendations.svelte` | **NEW** | Suggestion panel |
| `src/lib/canvas/heat.ts` | **NEW** | Heat dot rendering |

---

## Implementation Steps

### Step 1: Heat Tracking (~12k context)

1. Define heat calculation algorithm
2. Implement response analysis for block references
3. Track reference_count and last_referenced_turn
4. Calculate usage_heat with recency weighting
5. Unit tests for heat calculation

### Step 2: Position Relevance (~8k context)

1. Define position relevance algorithm
2. Implement zone-based scoring
3. Add age decay factor
4. Combine with usage_heat for overall score
5. Unit tests for relevance scoring

### Step 3: Heat Visualization (~12k context)

1. Create HeatOverlay canvas component
2. Implement dot density rendering
3. Implement dot color based on relevance
4. Add smooth transitions on updates
5. Integrate with ContextBlock

### Step 4: Topic Clustering (~10k context)

1. Implement keyword extraction
2. Implement overlap-based clustering algorithm
3. Add auto-labeling for clusters
4. Create cluster management UI
5. Unit tests for clustering

### Step 5: Deduplication (~8k context)

1. Implement exact duplicate detection
2. Implement near-duplicate detection (fuzzy matching)
3. Add consolidation logic
4. Create dedup suggestions UI
5. Unit tests for deduplication

---

## Test Coverage

### Unit Tests (~30 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/engine/heat.rs` | 8 | Heat calculation |
| `src-tauri/src/engine/attention.rs` | 6 | Reference detection |
| `src-tauri/src/engine/clustering.rs` | 8 | Clustering algorithm |
| `src-tauri/src/engine/dedup.rs` | 6 | Duplicate detection |
| `src-tauri/src/engine/recommendations.rs` | 2 | Suggestion generation |

### Integration Tests (~6 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_heat_flow.rs` | 3 | Request → response → heat update |
| `tests/integration/test_clustering.rs` | 3 | Clustering on real context |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_heat_visual` | Verify hot blocks show dense dots |
| `test_cold_fade` | Verify cold blocks visually fade |
| `test_cluster_display` | Verify cluster badges appear |
| `test_dedup_detection` | Read same file twice, verify detection |
| `test_recommendations` | Verify suggestions panel shows insights |
| `test_group_compress` | Select cluster, compress all |

---

## Success Criteria

- [ ] Usage heat calculated from response analysis
- [ ] Position relevance scored based on zone and age
- [ ] Heat visualization shows dot density correctly
- [ ] Topic clusters auto-detected and labeled
- [ ] Duplicates detected and flagged
- [ ] Recommendations surface actionable insights
- [ ] Heat transitions are smooth (60fps)
- [ ] Group operations work on clusters
- [ ] `make check` passes
- [ ] 30+ unit tests passing
- [ ] 6+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::engine::heat::{calculate_heat, HeatMap};
use crate::engine::clustering::{cluster_blocks, TopicCluster};
use crate::engine::dedup::{find_duplicates, DuplicateGroup};
use crate::engine::recommendations::generate_recommendations;
```
