# Phase 4: LLM-Driven Context Management with Metacognitive Cleaning

## Context

**Problem 1: Autonomous Archival Death Spiral**

Manual test session `72d1c60f` (Feb 18) revealed that autonomous archival is fundamentally broken:

1. Heuristics archive blocks automatically → removes important context (task instructions)
2. LLM forgets what it was doing → user observation: "I saw it follow a task, have context blocks get archived and it forget what it was doing"
3. Archival removes blocks from `messages[]` → shifts array indices → invalidates Anthropic's cache
4. Cache invalidation → ~50k `cache_create` tokens → budget spikes → triggers MORE archival → **death spiral**
5. Turns 10-14: 5 consecutive turns with 49,810 `cache_create` each (~$1.56 for just those 5 turns)

**Problem 2: Heuristics Don't Understand Importance**

Autonomous heuristics use staleness, zone, and budget pressure to decide what to archive. But they can't understand:
- Task context (what the user asked for)
- Cross-file dependencies (this old block explains that new one)
- Semantic relevance (this "stale" block is actually critical reference material)

**Only the LLM knows what's important.**

**Root architectural flaw:** We built metacognition tools (`aperture_context_*`) to give the LLM awareness and control, but then autonomous heuristics make decisions WITHOUT the LLM. This undermines the whole metacognition paradigm.

---

## Solution: LLM-Driven Context Management with Archival Suggestions

### Core Approach

**1. Disable Autonomous Archival**
- Heuristics NO LONGER apply archival automatically
- Heuristics become **suggestion generators** only
- Zero autonomous mutations → zero death spiral risk

**2. Surface Suggestions to LLM**
- **Threshold warnings** (breadcrumbs): "[Aperture: 60% util. 5 stale Middle-zone blocks suggested for archival]"
- **`aperture_context_preview` tool**: Add `suggested_archival: [{id, reason, staleness, zone, file_refs}]` field
- LLM sees suggestions as **auto-suggest** (like phone keyboard) — may or may not use them

**3. LLM Controls Archival via Staged Planning**
- LLM finishes task → evaluates if cleaning is worth it:
  - "Only 30% util, next task is simple → skip cleaning"
  - "60% util, next task is big → should clean first"
- LLM enters staged planning mode (`aperture_context_plan` with `stage`)
- LLM iteratively builds plan:
  - Review suggested archival → accept/reject/refine
  - Add compress actions
  - Add shift (zone moves)
  - Preview delta
- LLM commits when ready → **all mutations apply at once** → **one cache invalidation**

**4. User Confirmation for LLM-Initiated Cleaning** (optional)
- When LLM decides to clean mid-session: "I'm at 60%, next task is big. Should I clean context first? Any specific things to keep/remove?"
- User can provide hints (like `/compact` notes)
- LLM proceeds with guidance

**5. Breadcrumb Summary After Commit**
- After archival applied, LLM creates summary: "[Archived 5 old tool results from file exploration. Compressed 3 large bash outputs. Kept all task-related blocks.]"
- Breadcrumb stays in context → LLM remembers what it cleaned → no forgetting

**Why this solves the death spiral:**
- No autonomous archival → no runaway mutations
- LLM batches all context changes → one cache invalidation vs many
- LLM evaluates if cache cost is worth it (5k token cleaning → not worth it, 50k cleaning → worth it)
- LLM creates summary → remembers what was removed → no context amnesia
- User stays in control → can veto or guide cleaning decisions

---

## Implementation

### 1. Disable Autonomous Heuristics Execution

**File:** `src-tauri/src/engine/planner/mod.rs`

**Change `plan()` method (around line 365-390):**

REMOVE the heuristics execution block entirely:
```rust
// OLD CODE (DELETE):
if staged_mode_active {
    debug!("Staged plan active; skipping autonomous heuristics this turn");
} else if batch_point {
    let heuristic_mutations = heuristics::apply_heuristics(...);
    if !heuristic_mutations.is_empty() {
        debug!("Batch point: applying {} heuristic mutations", heuristic_mutations.len());
    }
    mutations.extend(heuristic_mutations);
} else {
    debug!("No batch point this turn; deferring heuristic mutations");
}
```

NEW CODE (autonomous heuristics NEVER auto-execute):
```rust
// Heuristics are now suggestion-only (never auto-execute)
// LLM controls all mutations via aperture_context_plan
debug!("Autonomous heuristics disabled — LLM controls context via staged planning");
```

**Keep all model-driven mutations** (from `pending_plan`) — those still execute.

### 2. Add Archival Suggestion Generation

**File:** `src-tauri/src/engine/planner/mod.rs`

Add new method to `ContextPlanner`:
```rust
/// Generate archival suggestions (non-executing) based on heuristics.
/// Returns candidate block IDs with metadata for LLM review.
pub fn generate_archival_suggestions(
    &self,
    blocks: &[Block],
    budget: &BudgetStatus,
) -> Vec<ArchivalSuggestion> {
    let mut suggestions = Vec::new();

    let effective_config = self.effective_config();
    let signals = self.build_heuristic_signals(blocks, budget, Vec::new());

    // Use existing heuristics logic, but DON'T apply mutations
    // Just collect candidates
    let candidates = heuristics::collect_archival_candidates(
        blocks,
        budget,
        &signals,
        &effective_config,
    );

    for candidate in candidates {
        suggestions.push(ArchivalSuggestion {
            block_id: candidate.id.clone(),
            reason: candidate.reason, // "stale (15 turns)", "low relevance", etc.
            staleness_turns: candidate.staleness,
            zone: candidate.zone,
            file_refs: candidate.metadata.file_paths.clone(),
            tokens: candidate.tokens,
        });
    }

    suggestions
}
```

Add new type in `src-tauri/src/engine/planner/types.rs`:
```rust
#[derive(Debug, Clone, Serialize)]
pub struct ArchivalSuggestion {
    pub block_id: String,
    pub reason: String,         // "stale (15 turns)", "low relevance", etc.
    pub staleness_turns: u32,
    pub zone: Zone,
    pub file_refs: Vec<String>,
    pub tokens: u32,
}
```

**File:** `src-tauri/src/engine/planner/heuristics.rs`

Add new function (extract from `apply_heuristics`):
```rust
/// Collect archival candidates WITHOUT applying mutations.
/// Returns list of candidates with metadata for LLM review.
pub fn collect_archival_candidates(
    blocks: &[Block],
    budget: &BudgetStatus,
    signals: &HeuristicSignals,
    config: &PlannerConfig,
) -> Vec<ArchivalCandidate> {
    let mut candidates = Vec::new();

    // Use existing staleness/pressure logic
    let staleness_config = StalenessConfig::default();
    let ranked = rank_by_staleness(blocks, &staleness_config);

    for block in ranked {
        if should_suggest_archival(block, budget, config) {
            candidates.push(ArchivalCandidate {
                id: block.id.clone(),
                reason: format_reason(block, signals),
                staleness: block.staleness_score(),
                zone: block.zone.clone(),
                metadata: block.metadata.clone(),
                tokens: block.tokens,
            });
        }
    }

    candidates
}

fn should_suggest_archival(
    block: &Block,
    budget: &BudgetStatus,
    config: &PlannerConfig,
) -> bool {
    // Don't suggest primacy or pinned
    if matches!(block.zone, Zone::BuiltIn(BuiltInZone::Primacy)) || block.pinned {
        return false;
    }

    // Suggest if stale AND (in middle zone OR high budget pressure)
    let is_stale = block.staleness_score() >= StalenessConfig::default().turn_threshold;
    let is_middle = matches!(block.zone, Zone::BuiltIn(BuiltInZone::Middle));
    let high_pressure = budget.utilization >= config.medium_utilization();

    is_stale && (is_middle || high_pressure)
}
```

### 3. Surface Suggestions in Threshold Warnings

**File:** `src-tauri/src/engine/planner/mod.rs`

Modify `check_alert_level_change()` to include suggestion count:
```rust
pub fn check_alert_level_change(&self, budget: &BudgetStatus, blocks: &[Block]) -> Option<String> {
    let current = self.pressure_level(budget.utilization);
    let mut guard = self.last_alert_level.lock().expect("last_alert_level lock");
    let previous = *guard;

    if current == previous {
        return None;
    }

    *guard = current;

    if current <= previous {
        return None; // Recovery, silent
    }

    // Generate suggestions
    let suggestions = self.generate_archival_suggestions(blocks, budget);
    let suggestion_count = suggestions.len();
    let suggestion_tokens: u32 = suggestions.iter().map(|s| s.tokens).sum();

    let pct = (budget.utilization * 100.0) as u32;
    let remaining = budget.remaining_tokens;
    let ceiling_pct = (self.effective_budget_ceiling() * 100.0) as u32;

    let message = match current {
        AlertLevel::Warning => format!(
            "[Aperture: context at {pct}% (soft threshold of {ceiling_pct}% ceiling) — {remaining} tokens remaining. {} stale blocks (~{} tokens) suggested for archival. Consider cleaning after current task.]",
            suggestion_count, suggestion_tokens
        ),
        AlertLevel::Critical => format!(
            "[Aperture: context at {pct}% (medium threshold of {ceiling_pct}% ceiling) — {remaining} tokens remaining. {} blocks (~{} tokens) suggested for archival. Pause and reorganize context now.]",
            suggestion_count, suggestion_tokens
        ),
        AlertLevel::Emergency => format!(
            "[Aperture: EMERGENCY — context at {pct}% (hard threshold = {ceiling_pct}% ceiling) — {remaining} tokens remaining. {} blocks MUST be archived to prevent overflow. Call aperture_context_plan immediately.]",
            suggestion_count, suggestion_tokens
        ),
        AlertLevel::Normal => return None,
    };

    Some(message)
}
```

**Update call site in `src-tauri/src/proxy/rewriter.rs` (line ~113):**
```rust
let budget_warning = engine.planner.check_alert_level_change(&budget, &blocks);
```

### 4. Add Suggestions to `aperture_context_preview` Tool

**File:** `src-tauri/src/metacog/tools.rs`

Modify `dispatch_tool()` for the `aperture_context_preview` case:
```rust
"aperture_context_preview" => {
    let blocks = engine.active_session_blocks();
    let budget = engine.budget_status();

    // Generate archival suggestions
    let suggestions = engine.planner.generate_archival_suggestions(&blocks, &budget);

    // Build preview with suggestions
    let preview = build_preview_with_suggestions(&blocks, &budget, &suggestions);

    serde_json::to_string(&preview).unwrap_or_else(|_| "{}".to_string())
}
```

Add helper function:
```rust
fn build_preview_with_suggestions(
    blocks: &[Block],
    budget: &BudgetStatus,
    suggestions: &[ArchivalSuggestion],
) -> Value {
    json!({
        "blocks": blocks.iter().map(|b| json!({
            "id": b.id,
            "role": b.role,
            "tokens": b.tokens,
            "zone": b.zone,
            "pinned": b.pinned,
            "summary": extract_summary(&b.content, 100),
        })).collect::<Vec<_>>(),
        "budget": {
            "used_tokens": budget.used_tokens,
            "limit_tokens": budget.limit_tokens,
            "utilization": format!("{:.1}%", budget.utilization * 100.0),
        },
        "suggested_archival": suggestions.iter().map(|s| json!({
            "block_id": s.block_id,
            "reason": s.reason,
            "staleness_turns": s.staleness_turns,
            "zone": s.zone,
            "tokens": s.tokens,
            "file_refs": s.file_refs,
        })).collect::<Vec<_>>(),
    })
}
```

### 5. Keep Staged Planning Workflow (Already Built)

**No changes needed** — Phase 3 already implemented:
- `aperture_context_plan` with `control: { op: "stage" | "append" | "preview" | "commit" | "discard" }`
- Plan accumulation across multiple tool calls
- Delta preview before commit
- Atomic commit of all mutations

**Just ensure LLM knows this is the ONLY way to modify context** — no autonomous heuristics.

### 6. Add LLM Reasoning Triggers (Future Enhancement)

**Not in this phase** — but consider for Phase 5:
- System prompt addition: "When you finish a task and context is >50%, evaluate if cleaning is needed before the next task."
- Threshold warnings already prompt: "Consider cleaning after current task"
- User can manually request: `/clean` or "should you clean context?"

### 7. Staff Review Areas

**Focus on correctness of existing code:**

1. **Batch gating logic** (`src-tauri/src/engine/planner/mod.rs`):
   - `is_batch_point()` vs `check_alert_level_change()` ordering ✓ (already verified as correct)
   - State tracking in `last_alert_level` ✓ (working as designed)
   - No changes needed — batch gating is NOT the problem

2. **Orphan tool sanitizers** (`src-tauri/src/proxy/rewriter.rs`):
   - `sanitize_anthropic_orphan_tool_results()` ✓ (Fix C from Feb 16)
   - `sanitize_anthropic_orphan_tool_uses()` ✓ (Fix C from Feb 16)
   - Both run in cold-start and main rewrite paths ✓

3. **Budget overhead tracking** (`src-tauri/src/proxy/parser.rs`, `src-tauri/src/engine/mod.rs`):
   - `estimate_tool_overhead()` extracts tool array size ✓ (Fix B from Feb 16)
   - `budget_status()` includes overhead ✓
   - Closes ~12pp budget gap ✓

4. **Zone assignment** (`src-tauri/src/engine/zone.rs`):
   - Thinking blocks removed from Primacy ✓ (Fix A from Feb 16)
   - Token-proportional assignment ✓

**Clean up:**
- Remove any debug logs that spam
- Remove commented-out code
- Verify all error paths fail-open
- Check for race conditions in state tracking (none found in exploration)

---

## Testing Strategy

### Unit Tests

**File:** `src-tauri/src/engine/planner/heuristics.rs`

```rust
#[test]
fn test_collect_archival_suggestions() {
    let blocks = vec![
        Block { id: "b1", staleness: 0, zone: Recency, ... },  // Not stale
        Block { id: "b2", staleness: 15, zone: Middle, ... },  // Stale, middle
        Block { id: "b3", staleness: 20, zone: Primacy, ... }, // Stale but primacy
        Block { id: "b4", staleness: 12, zone: Middle, ... },  // Stale, middle
    ];

    let budget = BudgetStatus { utilization: 0.45, ... }; // Above soft
    let config = PlannerConfig::default();
    let signals = HeuristicSignals::default();

    let suggestions = collect_archival_candidates(&blocks, &budget, &signals, &config);

    // Should suggest b2 and b4 (stale + middle), NOT b1 (not stale) or b3 (primacy)
    assert_eq!(suggestions.len(), 2);
    assert!(suggestions.iter().any(|s| s.id == "b2"));
    assert!(suggestions.iter().any(|s| s.id == "b4"));
}

#[test]
fn test_no_suggestions_at_low_pressure() {
    let blocks = vec![
        Block { id: "b1", staleness: 15, zone: Middle, ... },
    ];

    let budget = BudgetStatus { utilization: 0.25, ... }; // Below soft
    let config = PlannerConfig::default();

    let suggestions = collect_archival_candidates(&blocks, &budget, &signals, &config);

    // No suggestions at low pressure (even if blocks are stale)
    assert_eq!(suggestions.len(), 0);
}
```

**File:** `src-tauri/src/engine/planner/mod.rs`

```rust
#[test]
fn test_autonomous_heuristics_disabled() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![/* some blocks */];
    let budget = BudgetStatus { utilization: 0.70, ... }; // High pressure

    let input = PlannerInput {
        blocks,
        pending_plan: None,
        signals: HeuristicSignals::default(),
        budget,
        file_mutations: None,
    };

    let output = planner.plan(&input);

    // Autonomous heuristics disabled → zero archival mutations (unless from pending_plan)
    let archival_count = output.mutations.iter().filter(|m| matches!(m, ContextMutation::Archive { .. })).count();
    assert_eq!(archival_count, 0);
}
```

### Integration Tests

**File:** `src-tauri/tests/tool_lifecycle_integration.rs`

```rust
#[test]
fn test_llm_driven_archival_via_staged_plan() {
    let engine = ContextEngine::new_in_memory(None);

    // Ingest blocks
    let blocks = vec![/* 10 blocks */];
    engine.ingest(..., blocks, 0);

    // Get suggestions
    let budget = engine.budget_status();
    let suggestions = engine.planner.generate_archival_suggestions(&engine.active_session_blocks(), &budget);
    assert!(suggestions.len() > 0);

    // LLM stages archival plan (simulating aperture_context_plan tool call)
    engine.planner.stage_plan(ContextPlan {
        archive: suggestions.iter().take(3).map(|s| s.block_id.clone()).collect(),
        ..Default::default()
    });

    // Preview shows what will be archived
    let staged = engine.planner.preview_staged_plan(&engine.active_session_blocks());
    assert_eq!(staged.delta.blocks_archived, 3);

    // Commit applies mutations
    let plan = engine.planner.take_pending_plan().unwrap();
    // ... apply mutations via applicator
    // ... verify blocks removed
}
```

### Manual Cache Cost Validation

**Procedure:**
1. Build 200k token conversation (read 15-20 large files)
2. Trigger archival at 50% utilization (archive ~5-10 blocks)
3. Send 5 more requests
4. **Before fix:** Expect ~50k `cache_create` on turn after archival
5. **After fix:** Expect 0 additional `cache_create` (cache stable)
6. Check Anthropic API usage in response headers

---

## Verification Checklist

### Code Changes
- [ ] Autonomous heuristics execution REMOVED from `plan()` method
- [ ] `collect_archival_candidates()` added to `heuristics.rs` (suggestion-only)
- [ ] `generate_archival_suggestions()` added to planner
- [ ] `ArchivalSuggestion` type added to `planner/types.rs`
- [ ] `check_alert_level_change()` updated to include suggestion counts in warnings
- [ ] `aperture_context_preview` tool updated to include `suggested_archival` field
- [ ] Call site in `rewriter.rs` updated to pass `&blocks` to `check_alert_level_change()`

### Tests
- [ ] `collect_archival_suggestions()` generates correct candidates (unit test)
- [ ] No suggestions at low pressure (unit test)
- [ ] Autonomous heuristics never execute (unit test)
- [ ] Suggestions appear in `aperture_context_preview` (integration test)
- [ ] LLM-driven archival via staged plan works (integration test)
- [ ] All existing tests still pass (`cargo test` + `npx vitest run`)

### Manual Validation
- [ ] Run manual test Prompts 1+2 from `dev/active/phase-4-compression-readiness/manual-test-prompts.md`
- [ ] Verify NO autonomous archival happens (context grows naturally, no mid-task removals)
- [ ] Check threshold warnings include suggestion counts: "[Aperture: 60% util. 5 stale blocks (~10k tokens) suggested...]"
- [ ] Call `aperture_context_preview` → verify `suggested_archival` field is present with candidates
- [ ] Use `aperture_context_plan` to stage archival → preview → commit → verify blocks removed
- [ ] Monitor cache tokens → archival should happen ONCE at commit (not multiple times)
- [ ] Verify budget % matches Claude Code's `/context` within 5%
- [ ] Confirm no 400 errors from orphan tool blocks
- [ ] Confirm LLM doesn't forget what it was doing (archival summary breadcrumbs work)

### Staff Review
- [ ] Review batch gating logic for correctness (already verified as correct)
- [ ] Check state tracking in `is_batch_point()` vs `check_alert_level_change()`
- [ ] Verify no race conditions in planner state machine
- [ ] Clean up any debug noise or commented code
- [ ] Ensure all error paths fail-open (fallback to normal archival if filtering fails)

---

## Critical Files

| File | Change Type | Purpose |
|------|-------------|---------|
| `src-tauri/src/engine/planner/mod.rs` | Remove code + add method | Disable autonomous heuristics, add `generate_archival_suggestions()` |
| `src-tauri/src/engine/planner/heuristics.rs` | Add function | `collect_archival_candidates()` (suggestion-only) |
| `src-tauri/src/engine/planner/types.rs` | Add type | `ArchivalSuggestion` struct |
| `src-tauri/src/metacog/tools.rs` | Modify tool | Add suggestions to `aperture_context_preview` output |
| `src-tauri/src/proxy/rewriter.rs` | Update call site | Pass `&blocks` to `check_alert_level_change()` |
| `src-tauri/tests/tool_lifecycle_integration.rs` | Add tests | LLM-driven archival validation |

---

## Rollout Plan

### Phase A: Disable Autonomous Archival (Required)
1. Remove autonomous heuristics execution from `plan()` method
2. Add `collect_archival_candidates()` to heuristics
3. Add `generate_archival_suggestions()` to planner
4. Update threshold warnings to include suggestion counts
5. Run tests (verify autonomous heuristics never execute)

### Phase B: Surface Suggestions to LLM (Required)
1. Add `suggested_archival` field to `aperture_context_preview` tool
2. Test suggestions appear correctly
3. Manual validation: call preview tool, verify candidates are sensible

### Phase C: Staff Review & Clean Code (Required)
1. Review batch gating logic (already verified as correct)
2. Review orphan sanitizers (already working)
3. Review budget overhead tracking (already working)
4. Clean up debug noise, commented code
5. Verify error paths fail-open

### Phase D: Manual Test & Validation (Required)
1. Re-run manual test Prompts 1+2
2. Verify NO autonomous archival (context grows naturally)
3. Verify suggestions appear in warnings and preview tool
4. Test LLM-driven archival via staged planning workflow
5. Monitor cache stability (one invalidation at commit, not multiple)
6. Confirm no 400 errors, no forgetting, budget % matches `/context`

---

## Expected Outcomes

**Before fix (autonomous archival):**
- Turn 15: Budget crosses soft threshold (40%) → autonomous heuristics archive 3 blocks
- Cache invalidation → ~30k `cache_create` tokens → budget GROWS to 55%
- Turn 16: Budget crosses medium threshold (64%) → archive 10 more blocks
- Cache invalidation → ~50k `cache_create` → budget GROWS to 70%
- Turn 17: Budget crosses hard threshold (80%) → archive 20 blocks
- Death spiral: 3 archival events causing **$1.50-$2.00 cost** for supposedly "freeing" context
- LLM forgets task instructions → user observation: "it forget what it was doing"

**After fix (LLM-driven archival):**
- Turn 15: Budget crosses soft threshold (40%) → **warning injected**: "[Aperture: 40% util. 3 stale blocks (~5k tokens) suggested for archival]"
- **NO autonomous archival** → context grows naturally → budget increases to 55%
- Turn 20: Budget at 60%, LLM finishes task → **LLM evaluates**: "Next task is big, should I clean?"
- LLM calls `aperture_context_preview` → sees `suggested_archival: [{block_id, reason, tokens}]`
- LLM enters staged planning:
  - `aperture_context_plan({ op: "stage", archive: ["b2", "b5", "b8"], compress: [...] })`
  - `aperture_context_plan({ op: "preview" })` → reviews delta
  - `aperture_context_plan({ op: "commit" })` → **ALL mutations apply at once**
- **ONE cache invalidation** at commit → ~$0.30 for 50k tokens archived (vs $1.50-$2.00 in death spiral)
- LLM creates breadcrumb: "[Cleaned: archived 3 tool results, compressed 2 outputs. Kept task context.]"
- **LLM remembers** what was removed → no forgetting

**Cost savings:**
- Autonomous (broken): $1.50-$2.00 for 50k archived, 3+ invalidation events, causes death spiral
- LLM-driven (new): $0.30 for 50k archived, 1 invalidation event, controlled by LLM
- **5-7× cost reduction** + no death spiral + no forgetting

---

## Future Enhancements (Post-Phase 4)

1. **LLM reasoning triggers** - System prompt additions to guide when to clean
2. **User confirmation flow** - AskUserQuestion before LLM-initiated cleaning
3. **User-provided cleaning hints** - "Keep anything about authentication, remove old file reads"
4. **Suggestion quality improvements** - Better heuristics for what's actually archival-worthy
5. **Cleaning effectiveness metrics** - Track token savings vs cache cost, show ROI to LLM
