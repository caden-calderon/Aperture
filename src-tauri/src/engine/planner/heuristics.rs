//! Autonomous heuristics for context management.
//!
//! Always-running system-driven context management that operates
//! independently of model actions. Runs AFTER model mutations in
//! the planner pipeline — model intent takes priority.
//!
//! Three pressure levels derived from `PlannerConfig`:
//! - **Soft** (default 40% utilization): Archive stalest blocks (free ~5% of budget)
//! - **Medium** (default 64% utilization): Archive middle-zone stale blocks (free ~15%)
//! - **Hard** (default 80% utilization): Emergency — protect only primacy + recency

use std::collections::HashSet;

use tracing::debug;

use super::relevance;
use super::types::{
    ArchivalSuggestion, ArchivalSuggestionTier, ContextMutation, HeuristicSignals, PlannerConfig,
};
use crate::engine::block::Block;
use crate::engine::budget::{AlertLevel, BudgetStatus};
use crate::engine::staleness::{rank_by_staleness, StalenessConfig};
use crate::engine::types::{BuiltInZone, Role, Zone};

/// Collect archival candidates WITHOUT applying mutations.
/// Returns list of candidates with metadata for LLM review.
///
/// This is the suggestion-only version of archival heuristics. Used by the
/// planner to generate `ArchivalSuggestion`s that are surfaced to the LLM
/// via `aperture_context_preview` tool. The LLM decides whether to accept,
/// reject, or refine these suggestions via staged planning.
pub fn collect_archival_candidates(
    blocks: &[Block],
    budget: &BudgetStatus,
    signals: &HeuristicSignals,
    config: &PlannerConfig,
) -> Vec<ArchivalSuggestion> {
    let mut tier_a = Vec::new();
    let mut tier_b = Vec::new();

    // Compute relevance boosts from current-turn file references
    let relevance_boosts = relevance::compute_relevance_boosts(blocks, &signals.current_turn_files);

    let staleness_config = StalenessConfig::default();
    let current_turn = signals.current_turn;

    // Determine pressure level
    let pressure_level = if budget.utilization >= config.hard_utilization() {
        AlertLevel::Emergency
    } else if budget.utilization >= config.medium_utilization() {
        AlertLevel::Critical
    } else if budget.utilization >= config.soft_utilization() {
        AlertLevel::Warning
    } else {
        AlertLevel::Normal
    };

    // Get candidates: unpinned, not boosted by relevance
    let ranked = rank_by_staleness(blocks, current_turn + 1, &staleness_config);

    let allow_tier_b = signals.task_boundary_detected && pressure_level >= AlertLevel::Critical;

    for (block_id, _score) in ranked {
        let Some(block) = blocks.iter().find(|b| b.id == block_id) else {
            continue;
        };

        // Skip if not archival candidate
        if !is_archival_candidate(blocks, &block_id, &relevance_boosts, pressure_level) {
            continue;
        }

        let turns_since =
            current_turn.saturating_sub(block.last_referenced_turn.max(block.metadata.turn_index));
        let is_stale = turns_since >= config.staleness_turn_threshold;

        // Tier A (default): stale + middle-zone only.
        if block.zone == Zone::BuiltIn(BuiltInZone::Middle) && is_stale {
            tier_a.push(ArchivalSuggestion {
                block_id: block.id.clone(),
                tier: ArchivalSuggestionTier::TierA,
                reason: format!("stale ({} turns)", turns_since),
                staleness_turns: turns_since,
                zone: block.zone.clone(),
                file_refs: block.metadata.file_paths.clone(),
                tokens: block.tokens,
            });
            continue;
        }

        // Tier B (opportunistic): recency candidates only when all gates pass.
        if allow_tier_b && block.zone == Zone::BuiltIn(BuiltInZone::Recency) {
            tier_b.push(ArchivalSuggestion {
                block_id: block.id.clone(),
                tier: ArchivalSuggestionTier::TierB,
                reason: "task-boundary pressure: low-relevance recency".to_string(),
                staleness_turns: turns_since,
                zone: block.zone.clone(),
                file_refs: block.metadata.file_paths.clone(),
                tokens: block.tokens,
            });
        }
    }

    tier_a.extend(tier_b);
    tier_a
}

/// Apply autonomous heuristics to generate additional mutations.
///
/// Called AFTER model mutations are collected. Any blocks the model
/// explicitly pinned or otherwise acted on are protected from heuristic
/// archival via `model_acted_block_ids`.
pub fn apply_heuristics(
    blocks: &[Block],
    budget: &BudgetStatus,
    signals: &HeuristicSignals,
    config: &PlannerConfig,
    model_acted_block_ids: &HashSet<String>,
) -> Vec<ContextMutation> {
    let mut mutations = Vec::new();

    // Compute relevance boosts from current-turn file references
    let relevance_boosts = relevance::compute_relevance_boosts(blocks, &signals.current_turn_files);

    // Budget pressure archival
    let pressure_mutations = apply_budget_pressure(
        blocks,
        budget,
        config,
        model_acted_block_ids,
        &relevance_boosts,
    );
    mutations.extend(pressure_mutations);

    // Staleness-driven archival (only blocks exceeding turn threshold)
    let staleness_mutations = apply_staleness_archival(
        blocks,
        signals,
        config,
        model_acted_block_ids,
        &relevance_boosts,
        &mutations, // Don't double-archive blocks already targeted
    );
    mutations.extend(staleness_mutations);

    if !mutations.is_empty() {
        debug!(
            "Heuristics generated {} mutations (budget: {:.0}%, threshold: soft={:.0}%)",
            mutations.len(),
            budget.utilization * 100.0,
            config.soft_utilization() * 100.0,
        );
    }

    mutations
}

/// Apply budget-pressure-based archival.
///
/// Progressive archival depending on how close we are to the budget ceiling:
/// - Below soft: no action
/// - Soft → medium: archive stalest blocks to free ~5% of budget limit
/// - Medium → hard: archive stale middle-zone blocks to free ~15% of budget limit
/// - Above hard: emergency archival — archive everything except primacy + recency
fn apply_budget_pressure(
    blocks: &[Block],
    budget: &BudgetStatus,
    config: &PlannerConfig,
    protected_ids: &HashSet<String>,
    relevance_boosts: &std::collections::HashMap<String, f64>,
) -> Vec<ContextMutation> {
    let utilization = budget.utilization;

    if utilization < config.soft_utilization() {
        return vec![];
    }

    let staleness_config = StalenessConfig::default();
    let current_turn = blocks
        .iter()
        .map(|b| b.metadata.turn_index)
        .max()
        .unwrap_or(0);

    // Determine pressure level for tool-block archival decisions
    let pressure_level = if utilization >= config.hard_utilization() {
        AlertLevel::Emergency
    } else if utilization >= config.medium_utilization() {
        AlertLevel::Critical
    } else {
        AlertLevel::Warning
    };

    // Get candidates: unpinned, not protected by model, not boosted by relevance
    let ranked = rank_by_staleness(blocks, current_turn + 1, &staleness_config);

    let candidates: Vec<&str> = ranked
        .iter()
        .filter(|(id, _score)| {
            !protected_ids.contains(id)
                && is_archival_candidate(blocks, id, relevance_boosts, pressure_level)
        })
        .map(|(id, _)| id.as_str())
        .collect();

    if utilization >= config.hard_utilization() {
        // Hard pressure: archive everything except primacy + recency zone blocks
        debug!(
            "Hard budget pressure ({:.0}%) — emergency archival",
            utilization * 100.0
        );
        candidates
            .into_iter()
            .filter(|id| {
                blocks
                    .iter()
                    .any(|b| b.id == *id && b.zone == Zone::BuiltIn(BuiltInZone::Middle))
            })
            .map(|id| ContextMutation::Archive {
                block_id: id.to_string(),
            })
            .collect()
    } else if utilization >= config.medium_utilization() {
        // Medium pressure: archive stale middle-zone blocks to free ~15% of budget
        debug!(
            "Medium budget pressure ({:.0}%) — archiving stale middle blocks",
            utilization * 100.0
        );
        let target_tokens = (budget.limit_tokens as f64 * MEDIUM_TARGET_FRACTION) as u32;
        accumulate_archival_candidates(blocks, candidates, target_tokens, true)
    } else {
        // Soft pressure: archive stalest blocks to free ~5% of budget
        debug!(
            "Soft budget pressure ({:.0}%) — archiving stalest blocks",
            utilization * 100.0
        );
        let target_tokens = (budget.limit_tokens as f64 * SOFT_TARGET_FRACTION) as u32;
        accumulate_archival_candidates(blocks, candidates, target_tokens, false)
    }
}

/// Walk candidates in staleness order, accumulating token savings until target is met.
fn accumulate_archival_candidates(
    blocks: &[Block],
    candidates: Vec<&str>,
    target_tokens: u32,
    middle_only: bool,
) -> Vec<ContextMutation> {
    let mut mutations = Vec::new();
    let mut accumulated: u32 = 0;

    for id in candidates {
        if accumulated >= target_tokens {
            break;
        }
        let Some(block) = blocks.iter().find(|b| b.id == id) else {
            continue;
        };
        if middle_only && block.zone != Zone::BuiltIn(BuiltInZone::Middle) {
            continue;
        }
        accumulated = accumulated.saturating_add(block.tokens);
        mutations.push(ContextMutation::Archive {
            block_id: id.to_string(),
        });
    }

    mutations
}

/// Apply staleness-driven archival for blocks exceeding the turn threshold.
///
/// Blocks not referenced in N turns (configurable, default 10) are candidates
/// for archival, regardless of budget pressure.
fn apply_staleness_archival(
    blocks: &[Block],
    signals: &HeuristicSignals,
    config: &PlannerConfig,
    protected_ids: &HashSet<String>,
    relevance_boosts: &std::collections::HashMap<String, f64>,
    already_archived: &[ContextMutation],
) -> Vec<ContextMutation> {
    let already_archived_ids: HashSet<&str> = already_archived
        .iter()
        .filter_map(|m| match m {
            ContextMutation::Archive { block_id } => Some(block_id.as_str()),
            _ => None,
        })
        .collect();

    let mut mutations = Vec::new();

    for block in blocks {
        // Skip protected, already archived, pinned, thinking, or primacy/recency blocks
        if protected_ids.contains(&block.id)
            || already_archived_ids.contains(block.id.as_str())
            || block.pinned.is_some()
            || block.role == Role::Thinking
            || block.role == Role::ToolUse
            || block.role == Role::ToolResult
            || block.zone == Zone::BuiltIn(BuiltInZone::Primacy)
            || block.zone == Zone::BuiltIn(BuiltInZone::Recency)
        {
            continue;
        }

        // Skip blocks with relevance boosts from current-turn file references
        if relevance_boosts.contains_key(&block.id) {
            continue;
        }

        // Check if block exceeds staleness turn threshold
        let turns_since = signals
            .current_turn
            .saturating_sub(block.last_referenced_turn.max(block.metadata.turn_index));

        if turns_since >= config.staleness_turn_threshold {
            mutations.push(ContextMutation::Archive {
                block_id: block.id.clone(),
            });
        }
    }

    mutations
}

/// Check if a block is a valid archival candidate.
///
/// Excludes: pinned blocks, primacy/recency zone blocks, and blocks
/// with high relevance boosts from current-turn file references.
///
/// Tool lifecycle blocks (ToolUse/ToolResult) are only archivable at
/// Critical/Emergency pressure AND when in the Middle zone.
fn is_archival_candidate(
    blocks: &[Block],
    block_id: &str,
    relevance_boosts: &std::collections::HashMap<String, f64>,
    pressure_level: AlertLevel,
) -> bool {
    let Some(block) = blocks.iter().find(|b| b.id == block_id) else {
        return false;
    };

    // Pinned blocks are never candidates
    if block.pinned.is_some() {
        return false;
    }

    // Thinking blocks are never candidates — Anthropic requires byte-identical preservation
    if block.role == Role::Thinking {
        return false;
    }

    // Primacy zone blocks are protected (system prompts, etc.)
    if block.zone == Zone::BuiltIn(BuiltInZone::Primacy) {
        return false;
    }

    // Tool lifecycle blocks: only archivable at Critical+ pressure in Middle zone
    if block.role == Role::ToolUse || block.role == Role::ToolResult {
        if pressure_level < AlertLevel::Critical {
            return false;
        }
        if block.zone != Zone::BuiltIn(BuiltInZone::Middle) {
            return false;
        }
    }

    // Blocks with strong relevance boosts resist archival
    if let Some(&boost) = relevance_boosts.get(block_id) {
        if boost >= RELEVANCE_ARCHIVAL_THRESHOLD {
            return false;
        }
    }

    true
}

/// Target fraction of budget limit to free at soft pressure (~5%).
const SOFT_TARGET_FRACTION: f64 = 0.05;

/// Target fraction of budget limit to free at medium pressure (~15%).
const MEDIUM_TARGET_FRACTION: f64 = 0.15;

/// Minimum relevance boost that prevents archival.
const RELEVANCE_ARCHIVAL_THRESHOLD: f64 = 0.3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::budget::AlertLevel;
    use crate::engine::types::{CompressionLevel, PinPosition, Role};

    fn make_block_full(
        id: &str,
        zone: BuiltInZone,
        tokens: u32,
        turn: u32,
        last_ref_turn: u32,
        file_paths: Vec<&str>,
    ) -> Block {
        Block {
            id: id.to_string(),
            role: Role::Assistant,
            block_type: None,
            content: format!("Content of block {id}"),
            tokens,
            timestamp: "2026-02-13T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(zone),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: format!("Content of block {id}"),
                    tokens,
                },
                trimmed: None,
                summarized: None,
                minimal: None,
            },
            usage_heat: 0.0,
            position_relevance: 0.0,
            last_referenced_turn: last_ref_turn,
            reference_count: 0,
            topic_cluster: None,
            topic_keywords: vec![],
            metadata: BlockMetadata {
                provider: "test".to_string(),
                turn_index: turn,
                tool_name: None,
                file_paths: file_paths.into_iter().map(String::from).collect(),
            },
        }
    }

    fn make_block(id: &str, zone: BuiltInZone, tokens: u32, turn: u32) -> Block {
        make_block_full(id, zone, tokens, turn, 0, vec![])
    }

    fn make_block_with_role(
        id: &str,
        role: Role,
        zone: BuiltInZone,
        tokens: u32,
        turn: u32,
    ) -> Block {
        let mut block = make_block(id, zone, tokens, turn);
        block.role = role;
        block
    }

    fn mock_budget(used: u32, limit: u32) -> BudgetStatus {
        let utilization = if limit > 0 {
            used as f64 / limit as f64
        } else {
            0.0
        };
        BudgetStatus {
            used_tokens: used,
            limit_tokens: limit,
            utilization,
            alert_level: if utilization >= 0.95 {
                AlertLevel::Emergency
            } else if utilization >= 0.90 {
                AlertLevel::Critical
            } else if utilization >= 0.80 {
                AlertLevel::Warning
            } else {
                AlertLevel::Normal
            },
            remaining_tokens: limit.saturating_sub(used),
        }
    }

    fn default_signals(current_turn: u32) -> HeuristicSignals {
        HeuristicSignals {
            current_turn,
            ..Default::default()
        }
    }

    #[test]
    fn test_collect_archival_candidates_tier_a_only_stale_middle() {
        let blocks = vec![
            make_block("middle_stale", BuiltInZone::Middle, 1200, 0),
            make_block("middle_recent", BuiltInZone::Middle, 1200, 9),
            make_block("recency", BuiltInZone::Recency, 1200, 9),
        ];
        let budget = mock_budget(20_000, 200_000); // below pressure thresholds
        let config = PlannerConfig {
            staleness_turn_threshold: 5,
            ..PlannerConfig::default()
        };
        let signals = default_signals(10);

        let suggestions = collect_archival_candidates(&blocks, &budget, &signals, &config);

        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].block_id, "middle_stale");
        assert!(suggestions[0].tier.is_primary());
    }

    #[test]
    fn test_collect_archival_candidates_tier_b_requires_boundary_and_critical_pressure() {
        let blocks = vec![make_block(
            "recency_old_task",
            BuiltInZone::Recency,
            1500,
            1,
        )];
        let config = PlannerConfig {
            staleness_turn_threshold: 100,
            ..PlannerConfig::default()
        };

        let baseline_budget = mock_budget(100_000, 200_000); // 50% < critical (64%)
        let baseline_signals = HeuristicSignals {
            current_turn: 10,
            task_boundary_detected: true,
            ..Default::default()
        };
        let baseline =
            collect_archival_candidates(&blocks, &baseline_budget, &baseline_signals, &config);
        assert!(
            baseline.is_empty(),
            "Tier B disabled below critical pressure"
        );

        let critical_budget = mock_budget(150_000, 200_000); // 75% >= critical
        let no_boundary_signals = HeuristicSignals {
            current_turn: 10,
            task_boundary_detected: false,
            ..Default::default()
        };
        let no_boundary =
            collect_archival_candidates(&blocks, &critical_budget, &no_boundary_signals, &config);
        assert!(
            no_boundary.is_empty(),
            "Tier B disabled without task boundary"
        );

        let tier_b =
            collect_archival_candidates(&blocks, &critical_budget, &baseline_signals, &config);
        assert_eq!(tier_b.len(), 1);
        assert!(!tier_b[0].tier.is_primary());
    }

    // ── Budget Pressure Tests ────────────────────────────────

    #[test]
    fn test_no_archival_below_soft_threshold() {
        let blocks = vec![
            make_block("b1", BuiltInZone::Middle, 1000, 0),
            make_block("b2", BuiltInZone::Middle, 1000, 1),
        ];
        // 20% utilization, soft threshold is 40%
        let budget = mock_budget(40_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(5),
            &config,
            &HashSet::new(),
        );

        assert!(mutations.is_empty());
    }

    #[test]
    fn test_soft_pressure_archives_stalest() {
        let blocks = vec![
            make_block("b_old", BuiltInZone::Middle, 2000, 0),
            make_block("b_mid", BuiltInZone::Middle, 1000, 3),
            make_block("b_new", BuiltInZone::Recency, 500, 8),
        ];
        // 45% utilization (above soft 40%, below medium 64%)
        let budget = mock_budget(90_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        // Should archive some blocks but not all
        assert!(!mutations.is_empty());
        // Should target stalest (oldest + largest) first
        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();
        assert!(archived_ids.contains(&"b_old"));
    }

    #[test]
    fn test_soft_pressure_budget_limits_by_tokens() {
        // Create blocks where token accumulation matters
        let blocks: Vec<Block> = (0..10)
            .map(|i| make_block(&format!("b{i}"), BuiltInZone::Middle, 500, i + 8))
            .collect();
        let budget = mock_budget(90_000, 200_000); // 45%
        let config = PlannerConfig {
            staleness_turn_threshold: 100, // High threshold — no staleness archival
            ..PlannerConfig::default()
        };

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(15),
            &config,
            &HashSet::new(),
        );

        // Soft target: 5% of 200k = 10k tokens. Each block is 500 tokens.
        // Should archive up to 20 blocks to hit 10k, but we only have 10 candidates.
        let archive_count = mutations
            .iter()
            .filter(|m| matches!(m, ContextMutation::Archive { .. }))
            .count();
        let archived_tokens: u32 = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => {
                    blocks.iter().find(|b| b.id == *block_id).map(|b| b.tokens)
                }
                _ => None,
            })
            .sum();
        // Should free at most the soft target (10k), blocks are 500 each
        assert!(
            archive_count > 0,
            "Should archive at least one block at soft pressure"
        );
        assert!(
            archived_tokens <= 10_500,
            "Soft pressure should free approximately 10k tokens, got {archived_tokens}"
        );
    }

    #[test]
    fn test_medium_pressure_archives_middle_zone() {
        let blocks = vec![
            make_block("sys", BuiltInZone::Primacy, 500, 0),
            make_block("m1", BuiltInZone::Middle, 1000, 1),
            make_block("m2", BuiltInZone::Middle, 1000, 2),
            make_block("m3", BuiltInZone::Middle, 1000, 3),
            make_block("recent", BuiltInZone::Recency, 500, 9),
        ];
        // 70% utilization (above medium 64%, below hard 80%)
        let budget = mock_budget(140_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        // Should archive middle-zone blocks only
        assert!(!archived_ids.contains(&"sys")); // primacy protected
        assert!(!archived_ids.contains(&"recent")); // recency protected
                                                    // At least some middle blocks should be archived
        assert!(!archived_ids.is_empty());
    }

    #[test]
    fn test_hard_pressure_archives_all_middle() {
        let blocks = vec![
            make_block("sys", BuiltInZone::Primacy, 500, 0),
            make_block("m1", BuiltInZone::Middle, 1000, 1),
            make_block("m2", BuiltInZone::Middle, 1000, 2),
            make_block("m3", BuiltInZone::Middle, 1000, 3),
            make_block("recent", BuiltInZone::Recency, 500, 9),
        ];
        // 85% utilization (above hard 80%)
        let budget = mock_budget(170_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        // All middle-zone blocks should be archived
        assert!(archived_ids.contains(&"m1"));
        assert!(archived_ids.contains(&"m2"));
        assert!(archived_ids.contains(&"m3"));
        // Primacy and recency protected
        assert!(!archived_ids.contains(&"sys"));
        assert!(!archived_ids.contains(&"recent"));
    }

    // ── Model Intent Override ────────────────────────────────

    #[test]
    fn test_model_pinned_blocks_protected() {
        let mut blocks = vec![
            make_block("b1", BuiltInZone::Middle, 2000, 0),
            make_block("b2", BuiltInZone::Middle, 1000, 1),
        ];
        blocks[0].pinned = Some(PinPosition::Top);

        let budget = mock_budget(90_000, 200_000); // soft pressure
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(!archived_ids.contains(&"b1")); // pinned = protected
    }

    #[test]
    fn test_model_acted_blocks_protected() {
        let blocks = vec![
            make_block("b1", BuiltInZone::Middle, 2000, 0),
            make_block("b2", BuiltInZone::Middle, 1000, 1),
        ];

        let budget = mock_budget(90_000, 200_000); // soft pressure
        let config = PlannerConfig::default();
        let model_acted = HashSet::from(["b1".to_string()]);

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &model_acted,
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(!archived_ids.contains(&"b1")); // model acted on it = protected
    }

    // ── Relevance Boosting ───────────────────────────────────

    #[test]
    fn test_relevance_boost_protects_from_archival() {
        let blocks = vec![
            make_block_full("b1", BuiltInZone::Middle, 2000, 0, 0, vec!["src/auth.rs"]),
            make_block("b2", BuiltInZone::Middle, 1000, 1),
        ];

        let budget = mock_budget(90_000, 200_000); // soft pressure
        let config = PlannerConfig::default();
        let signals = HeuristicSignals {
            current_turn: 10,
            current_turn_files: vec!["src/auth.rs".to_string()],
            ..Default::default()
        };

        let mutations = apply_heuristics(&blocks, &budget, &signals, &config, &HashSet::new());

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        // b1 references auth.rs which is a current-turn file → boosted → protected
        assert!(!archived_ids.contains(&"b1"));
    }

    // ── Staleness Archival ───────────────────────────────────

    #[test]
    fn test_staleness_threshold_archives_old_blocks() {
        let blocks = vec![
            make_block("old", BuiltInZone::Middle, 500, 0),
            make_block("recent", BuiltInZone::Recency, 500, 19),
        ];

        // Below soft budget threshold → no budget pressure
        let budget = mock_budget(20_000, 200_000);
        let config = PlannerConfig {
            staleness_turn_threshold: 10,
            ..Default::default()
        };

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(20),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        // old block: turn 0, current turn 20, last_ref 0 → 20 turns since > 10 threshold
        assert!(archived_ids.contains(&"old"));
        // recent block: recency zone → protected
        assert!(!archived_ids.contains(&"recent"));
    }

    #[test]
    fn test_staleness_respects_last_referenced_turn() {
        let mut block = make_block("b1", BuiltInZone::Middle, 500, 0);
        block.last_referenced_turn = 15; // Referenced recently

        let blocks = vec![block];
        let budget = mock_budget(20_000, 200_000);
        let config = PlannerConfig {
            staleness_turn_threshold: 10,
            ..Default::default()
        };

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(20),
            &config,
            &HashSet::new(),
        );

        // Turn 0 created, last ref turn 15, current turn 20 → 5 turns since < 10 threshold
        assert!(mutations.is_empty());
    }

    #[test]
    fn test_staleness_does_not_double_archive() {
        // Block that would be caught by both budget pressure AND staleness
        let blocks = vec![make_block("b1", BuiltInZone::Middle, 2000, 0)];
        // Above soft threshold
        let budget = mock_budget(90_000, 200_000);
        let config = PlannerConfig {
            staleness_turn_threshold: 5,
            ..Default::default()
        };

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(20),
            &config,
            &HashSet::new(),
        );

        // Should only appear once, not double-archived
        let archive_count = mutations
            .iter()
            .filter(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "b1"))
            .count();
        assert_eq!(archive_count, 1);
    }

    // ── Primacy Protection ───────────────────────────────────

    #[test]
    fn test_primacy_blocks_never_archived() {
        let blocks = vec![
            make_block("sys", BuiltInZone::Primacy, 5000, 0),
            make_block("m1", BuiltInZone::Middle, 1000, 1),
        ];
        // Hard pressure
        let budget = mock_budget(170_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(!archived_ids.contains(&"sys"));
    }

    #[test]
    fn test_recency_blocks_never_archived_by_budget() {
        let blocks = vec![
            make_block("recent1", BuiltInZone::Recency, 2000, 8),
            make_block("recent2", BuiltInZone::Recency, 2000, 9),
        ];
        // Hard pressure
        let budget = mock_budget(170_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        // No middle-zone blocks to archive, so nothing archived
        assert!(mutations.is_empty());
    }

    // ── Tool Lifecycle Block Archival ─────────────────────────

    #[test]
    fn test_tool_lifecycle_blocks_protected_at_soft_pressure() {
        let blocks = vec![
            make_block_with_role("tool_use", Role::ToolUse, BuiltInZone::Middle, 500, 1),
            make_block_with_role("tool_result", Role::ToolResult, BuiltInZone::Middle, 500, 2),
            make_block("assistant_regular", BuiltInZone::Middle, 1500, 0),
        ];

        // Soft pressure — tool blocks should be protected
        let budget = mock_budget(90_000, 200_000);
        let config = PlannerConfig::default();
        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(20),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(!archived_ids.contains(&"tool_use"));
        assert!(!archived_ids.contains(&"tool_result"));
        assert!(archived_ids.contains(&"assistant_regular"));
    }

    #[test]
    fn test_tool_lifecycle_blocks_archived_at_hard_pressure() {
        let blocks = vec![
            make_block_with_role("tool_use", Role::ToolUse, BuiltInZone::Middle, 5000, 1),
            make_block_with_role(
                "tool_result",
                Role::ToolResult,
                BuiltInZone::Middle,
                10_000,
                2,
            ),
            make_block("assistant_regular", BuiltInZone::Middle, 1500, 0),
        ];

        // Hard pressure (>= 80% utilization) — Middle-zone tool blocks should be archived
        let budget = mock_budget(170_000, 200_000);
        let config = PlannerConfig::default();
        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(20),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            archived_ids.contains(&"tool_use"),
            "Middle-zone tool_use should be archived at hard pressure"
        );
        assert!(
            archived_ids.contains(&"tool_result"),
            "Middle-zone tool_result should be archived at hard pressure"
        );
        assert!(archived_ids.contains(&"assistant_regular"));
    }

    #[test]
    fn test_tool_lifecycle_blocks_in_recency_still_protected() {
        let blocks = vec![
            make_block_with_role("tool_use", Role::ToolUse, BuiltInZone::Recency, 5000, 8),
            make_block_with_role(
                "tool_result",
                Role::ToolResult,
                BuiltInZone::Recency,
                10_000,
                9,
            ),
        ];

        // Hard pressure — but Recency-zone tool blocks should still be safe
        let budget = mock_budget(190_000, 200_000);
        let config = PlannerConfig::default();
        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(20),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            !archived_ids.contains(&"tool_use"),
            "Recency-zone tool blocks should be protected even at emergency pressure"
        );
        assert!(!archived_ids.contains(&"tool_result"));
    }

    // ── Edge Cases ───────────────────────────────────────────

    #[test]
    fn test_empty_blocks_no_crash() {
        let blocks: Vec<Block> = vec![];
        let budget = mock_budget(100_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        assert!(mutations.is_empty());
    }

    #[test]
    fn test_thinking_blocks_never_archived() {
        let blocks = vec![
            make_block_with_role("thinking1", Role::Thinking, BuiltInZone::Middle, 2000, 1),
            make_block("assistant1", BuiltInZone::Middle, 500, 2),
        ];
        // Hard pressure — everything archivable should be archived
        let budget = mock_budget(170_000, 200_000);
        let config = PlannerConfig::default();

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(20),
            &config,
            &HashSet::new(),
        );

        let archived_ids: Vec<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            !archived_ids.contains(&"thinking1"),
            "Thinking blocks must never be archived"
        );
        assert!(archived_ids.contains(&"assistant1"));
    }

    #[test]
    fn test_thinking_blocks_excluded_from_archival_candidates() {
        let blocks = vec![
            make_block_with_role("thinking1", Role::Thinking, BuiltInZone::Middle, 2000, 0),
            make_block("assistant1", BuiltInZone::Middle, 500, 0),
        ];
        let budget = mock_budget(90_000, 200_000);
        let config = PlannerConfig {
            staleness_turn_threshold: 5,
            ..PlannerConfig::default()
        };

        let suggestions =
            collect_archival_candidates(&blocks, &budget, &default_signals(10), &config);

        let suggestion_ids: Vec<&str> = suggestions.iter().map(|s| s.block_id.as_str()).collect();
        assert!(
            !suggestion_ids.contains(&"thinking1"),
            "Thinking blocks must not appear in archival candidates"
        );
    }

    #[test]
    fn test_custom_config_thresholds() {
        let blocks = vec![
            make_block("b1", BuiltInZone::Middle, 1000, 0),
            make_block("b2", BuiltInZone::Middle, 1000, 1),
        ];

        // Custom config: ceiling 60%, soft at 30% of ceiling = 18%
        let config = PlannerConfig {
            budget_ceiling: 0.60,
            soft_threshold: 0.30,
            medium_threshold: 0.70,
            hard_threshold: 1.00,
            staleness_turn_threshold: 10,
            manifest_enabled: true,
        };

        // 20% utilization — above custom soft (18%)
        let budget = mock_budget(40_000, 200_000);

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(10),
            &config,
            &HashSet::new(),
        );

        assert!(!mutations.is_empty()); // Should trigger soft archival
    }

    #[test]
    fn test_token_based_medium_pressure_frees_proportional_tokens() {
        // Create many small blocks in Middle zone
        let blocks: Vec<Block> = (0..50)
            .map(|i| make_block(&format!("b{i}"), BuiltInZone::Middle, 1000, i))
            .collect();

        // 70% utilization (medium pressure)
        let budget = mock_budget(140_000, 200_000);
        let config = PlannerConfig {
            staleness_turn_threshold: 100, // disable staleness
            ..PlannerConfig::default()
        };

        let mutations = apply_heuristics(
            &blocks,
            &budget,
            &default_signals(60),
            &config,
            &HashSet::new(),
        );

        let archived_tokens: u32 = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => {
                    blocks.iter().find(|b| b.id == *block_id).map(|b| b.tokens)
                }
                _ => None,
            })
            .sum();

        // Medium target: 15% of 200k = 30k tokens
        // Should free approximately 30k (30 blocks * 1000 tokens each)
        assert!(
            archived_tokens >= 29_000 && archived_tokens <= 31_000,
            "Medium pressure should free ~30k tokens, got {archived_tokens}"
        );
    }
}
