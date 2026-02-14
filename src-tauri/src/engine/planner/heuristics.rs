//! Autonomous heuristics for context management.
//!
//! Always-running system-driven context management that operates
//! independently of model actions. Runs AFTER model mutations in
//! the planner pipeline — model intent takes priority.
//!
//! Three pressure levels derived from `PlannerConfig`:
//! - **Soft** (default 40% utilization): Archive stalest blocks
//! - **Medium** (default 64% utilization): Archive middle-zone stale blocks
//! - **Hard** (default 80% utilization): Emergency — protect only primacy + recency

use std::collections::HashSet;

use tracing::debug;

use super::relevance;
use super::types::{ContextMutation, HeuristicSignals, PlannerConfig};
use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;
use crate::engine::staleness::{rank_by_staleness, StalenessConfig};
use crate::engine::types::{BuiltInZone, Zone};

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
/// - Soft → medium: archive stalest unpinned blocks (up to 3)
/// - Medium → hard: archive all stale middle-zone blocks
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

    // Get candidates: unpinned, not protected by model, not boosted by relevance
    let ranked = rank_by_staleness(blocks, current_turn + 1, &staleness_config);

    let candidates: Vec<&str> = ranked
        .iter()
        .filter(|(id, _score)| {
            !protected_ids.contains(id) && is_archival_candidate(blocks, id, relevance_boosts)
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
        // Medium pressure: archive stale middle-zone blocks
        debug!(
            "Medium budget pressure ({:.0}%) — archiving stale middle blocks",
            utilization * 100.0
        );
        candidates
            .into_iter()
            .filter(|id| {
                blocks
                    .iter()
                    .any(|b| b.id == *id && b.zone == Zone::BuiltIn(BuiltInZone::Middle))
            })
            .take(MAX_MEDIUM_ARCHIVAL)
            .map(|id| ContextMutation::Archive {
                block_id: id.to_string(),
            })
            .collect()
    } else {
        // Soft pressure: archive stalest blocks (limited count)
        debug!(
            "Soft budget pressure ({:.0}%) — archiving stalest blocks",
            utilization * 100.0
        );
        candidates
            .into_iter()
            .take(MAX_SOFT_ARCHIVAL)
            .map(|id| ContextMutation::Archive {
                block_id: id.to_string(),
            })
            .collect()
    }
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
        // Skip protected, already archived, pinned, or primacy/recency blocks
        if protected_ids.contains(&block.id)
            || already_archived_ids.contains(block.id.as_str())
            || block.pinned.is_some()
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
fn is_archival_candidate(
    blocks: &[Block],
    block_id: &str,
    relevance_boosts: &std::collections::HashMap<String, f64>,
) -> bool {
    let Some(block) = blocks.iter().find(|b| b.id == block_id) else {
        return false;
    };

    // Pinned blocks are never candidates
    if block.pinned.is_some() {
        return false;
    }

    // Primacy zone blocks are protected (system prompts, etc.)
    if block.zone == Zone::BuiltIn(BuiltInZone::Primacy) {
        return false;
    }

    // Blocks with strong relevance boosts resist archival
    if let Some(&boost) = relevance_boosts.get(block_id) {
        if boost >= RELEVANCE_ARCHIVAL_THRESHOLD {
            return false;
        }
    }

    true
}

/// Maximum blocks to archive at soft pressure.
const MAX_SOFT_ARCHIVAL: usize = 3;

/// Maximum blocks to archive at medium pressure.
const MAX_MEDIUM_ARCHIVAL: usize = 10;

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
        assert!(mutations.len() <= MAX_SOFT_ARCHIVAL);
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
    fn test_soft_pressure_budget_archives_limited() {
        // Use recent blocks that won't trigger staleness archival
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

        let archive_count = mutations
            .iter()
            .filter(|m| matches!(m, ContextMutation::Archive { .. }))
            .count();
        assert!(
            archive_count <= MAX_SOFT_ARCHIVAL,
            "Soft budget pressure should limit to {MAX_SOFT_ARCHIVAL} archives, got {archive_count}"
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
}
