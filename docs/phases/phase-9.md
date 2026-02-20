# Phase 9: Analytics & Visualization

**Status**: PENDING
**Goal**: Token analytics, timeline view, context replay, health score, and warning system
**Prerequisites**: Phase 8 complete
**Estimated Scope**: ~55k context

---

## Context from Phase 8

Phase 8 delivers:
- Full-text search (current + historical sessions)
- Semantic search via cleaner model
- Keyword-based selection workflow
- Natural language commands with preview
- Block annotations with search integration
- CLI `aperture exec` command

**Key imports:**
```rust
use crate::search::{SearchEngine, SearchQuery, SearchResult};
use crate::nlp::{parse_command, NLCommand};
use crate::engine::annotation::Annotation;
```

**Integration point:** Phase 9 adds visibility into usage and health.

---

## Problem Statement

1. **No cost visibility** — Don't know where tokens/money go
2. **No history visualization** — Can't see how context evolved
3. **No forecasting** — Don't know when we'll hit limits
4. **No health metric** — No single score for context quality
5. **No proactive warnings** — Problems discovered too late

---

## Deliverables

### 1. Token Analytics Dashboard

Detailed cost tracking:
```
Session Cost Breakdown:
  tool_result blocks: 67% of token spend ($1.57)
  assistant blocks:   22% ($0.52)
  user blocks:         8% ($0.19)
  system prompt:       3% ($0.07)

Insight: Average tool_result lifespan: 4 turns.
         Auto-condensing after 5 turns saves ~$0.89/session (38%).

Cost per block: schema.py read = $0.12/turn it stays in context
```

Features:
- Cost-per-block visibility
- Cross-session comparison
- Optimization suggestions with savings estimates
- Block type distribution charts

### 2. Token Cost Forecasting

Predict when you'll hit limits:
```
Current usage: 124,000 / 200,000 tokens (62%)
Growth rate: ~3,200 tokens/turn
Estimated limit hit: ~24 turns
Recommendation: Consider condensing middle zone (38,000 tokens recoverable)
```

Features:
- Live projection after every turn
- Growth rate calculation (rolling average)
- "Turns until limit" estimate
- Visual ghost fill on budget bar

### 3. Session Timeline View

Horizontal visualization of session evolution:
- Each block is a segment
- Segment height = token cost
- Segment color = block type / heat
- Halftone density = compression level
- Zoom from full session to individual turns
- Click segment to select block
- Dependency edges overlaid
- Checkpoint markers as vertical lines

### 3b. Multi-Window / Split View (All Essential)

All five split view modes are core Phase 9 features:

1. **Session comparison** — Two active sessions side by side
2. **Snapshot diff** — Two snapshots with diff highlighting
3. **Fork comparison** — Two branches of a forked context
4. **Timeline + Editor** — Timeline view on top, block editor below
5. **Before/After compression** — Original vs compressed versions

**Implementation:**
- Flexible pane system (horizontal/vertical split)
- Synchronized scrolling option
- Selection sync (select in one pane, highlight in other)
- Keyboard shortcuts for quick split/unsplit

### 4. Context Replay

Step through session evolution:
- Scrubber/timeline showing each state change
- See when blocks entered, compressed, moved, removed
- Token usage graph over time
- Filter by event type
- Export replay as report

Use case: "Identify when budget pressure started"

### 5. Context Health Score

Single composite number (0-100):
```
Context Health: 64/100

-23 pts  Dead weight in middle zone (18 blocks, zero heat)
 -8 pts  3 duplicate file reads (config.py read 3x)
 -5 pts  2 orphaned blocks (dependencies removed)

Suggested actions:
  Compress 12 stale blocks       → +15 pts
  Deduplicate config.py reads    → +8 pts
  Remove orphaned blocks         → +5 pts
```

Factors:
- Budget utilization
- Zone balance
- Heat distribution
- Compression ratio
- Deduplication opportunities
- Dependency integrity

### 6. Warning System

Configurable alerts:
- **Budget warnings**: "Context at 80% capacity"
- **Dead weight**: "5 blocks have zero heat for 10+ turns"
- **Compression quality**: "Ratio 15:1 — quality may be degraded"
- **Deduplication**: "3 near-duplicate file reads"
- **Orphan risk**: "Block #7 has 4 dependents"
- **Forecast**: "Budget limit in ~8 turns"

Configuration:
- Enable/disable per type
- Set thresholds
- Choose delivery (toast, system notification, badge)
- Auto-action option

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src-tauri/src/analytics/mod.rs` | **NEW** | Analytics module |
| `src-tauri/src/analytics/cost.rs` | **NEW** | Cost calculation |
| `src-tauri/src/analytics/forecast.rs` | **NEW** | Forecasting |
| `src-tauri/src/analytics/health.rs` | **NEW** | Health score |
| `src-tauri/src/analytics/timeline.rs` | **NEW** | Timeline data |
| `src-tauri/src/analytics/warnings.rs` | **NEW** | Warning system |
| `src/lib/components/AnalyticsDashboard.svelte` | **NEW** | Dashboard UI |
| `src/lib/components/TimelineView.svelte` | **NEW** | Timeline |
| `src/lib/components/ReplayControls.svelte` | **NEW** | Replay UI |
| `src/lib/components/HealthScore.svelte` | **NEW** | Health display |
| `src/lib/components/WarningToast.svelte` | **NEW** | Warning UI |

---

## Implementation Steps

### Step 1: Cost Analytics (~12k context)

1. Implement cost calculation per block
2. Add session cost aggregation
3. Create cost breakdown by type
4. Generate optimization suggestions
5. Build dashboard UI
6. Unit tests for calculations

### Step 2: Forecasting (~10k context)

1. Implement growth rate calculation
2. Create projection algorithm
3. Add "turns until limit" estimate
4. Build forecast visualization
5. Unit tests for forecasting

### Step 3: Timeline View (~12k context)

1. Create timeline data structure
2. Implement segment rendering
3. Add zoom/pan controls
4. Add dependency edge overlay
5. Integrate with canvas layer
6. Unit tests for timeline

### Step 4: Context Replay (~10k context)

1. Implement state history tracking
2. Create scrubber UI
3. Add event filtering
4. Implement export
5. Unit tests for replay

### Step 5: Health Score (~6k context)

1. Define health factors
2. Implement scoring algorithm
3. Create breakdown UI
4. Add suggested actions
5. Unit tests for scoring

### Step 6: Warning System (~5k context)

1. Define warning types
2. Implement threshold checking
3. Create notification system
4. Add configuration UI
5. Unit tests for warnings

---

## Test Coverage

### Unit Tests (~35 tests)

| File | Tests | Focus |
|------|-------|-------|
| `src-tauri/src/analytics/cost.rs` | 8 | Cost calculations |
| `src-tauri/src/analytics/forecast.rs` | 6 | Forecasting |
| `src-tauri/src/analytics/health.rs` | 8 | Health scoring |
| `src-tauri/src/analytics/timeline.rs` | 6 | Timeline data |
| `src-tauri/src/analytics/warnings.rs` | 7 | Warning triggers |

### Integration Tests (~6 tests)

| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_analytics.rs` | 3 | Analytics flow |
| `tests/integration/test_warnings.rs` | 3 | Warning triggers |

### Manual Tests (6 tests)

| Test | Description |
|------|-------------|
| `test_cost_dashboard` | View cost breakdown, verify accuracy |
| `test_forecast_display` | Check forecast, verify projection |
| `test_timeline_zoom` | Zoom timeline, verify segments |
| `test_replay_scrub` | Scrub through history, verify states |
| `test_health_breakdown` | Click health score, verify factors |
| `test_warning_trigger` | Hit 80% budget, verify warning |

---

## Success Criteria

- [ ] Cost analytics show accurate breakdown
- [ ] Forecasting predicts limit within reasonable accuracy
- [ ] Timeline view renders with zoom/pan
- [ ] Replay steps through session history
- [ ] Health score reflects context quality
- [ ] Warnings trigger at configured thresholds
- [ ] Dashboard is responsive and performant
- [ ] `make check` passes
- [ ] 35+ unit tests passing
- [ ] 6+ integration tests passing
- [ ] All manual tests documented and passing

---

## Key Imports for Next Phase

```rust
use crate::analytics::{CostAnalytics, Forecast, HealthScore, TimelineData};
use crate::analytics::warnings::{Warning, WarningConfig};
```
