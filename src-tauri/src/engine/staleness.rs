//! Staleness scoring for context blocks.
//!
//! Staleness measures how "stale" a block is — older, larger blocks
//! with fewer references are more stale and better candidates for
//! compression or removal.
//!
//! Formula: staleness = (turns_since_created × token_cost) / relevance_boost

use super::block::Block;

/// Staleness scoring configuration.
#[derive(Debug, Clone)]
pub struct StalenessConfig {
    /// Weight for age (turns since creation).
    pub age_weight: f64,
    /// Weight for token cost (normalized to 0-1).
    pub token_weight: f64,
    /// Base relevance boost (prevents division by zero).
    pub base_relevance: f64,
    /// Boost per reference from later blocks.
    pub reference_boost: f64,
    /// Maximum staleness score (clamped).
    pub max_score: f64,
}

impl Default for StalenessConfig {
    fn default() -> Self {
        Self {
            age_weight: 1.0,
            token_weight: 0.5,
            base_relevance: 1.0,
            reference_boost: 0.5,
            max_score: 100.0,
        }
    }
}

/// Calculate staleness score for a single block.
///
/// Higher score = more stale = better candidate for compression/removal.
pub fn calculate_staleness(block: &Block, current_turn: u32, config: &StalenessConfig) -> f64 {
    let turns_since_created = current_turn.saturating_sub(block.metadata.turn_index) as f64;
    let token_cost = block.tokens as f64 / 1000.0; // normalize to ~0-N range

    let age_factor = turns_since_created * config.age_weight;
    let cost_factor = token_cost * config.token_weight;

    let relevance_boost = config.base_relevance
        + (block.reference_count as f64 * config.reference_boost)
        + block.usage_heat;

    let staleness = (age_factor * cost_factor) / relevance_boost;

    staleness.min(config.max_score).max(0.0)
}

/// Score all blocks and return them sorted by staleness (most stale first).
pub fn rank_by_staleness(
    blocks: &[Block],
    current_turn: u32,
    config: &StalenessConfig,
) -> Vec<(String, f64)> {
    let mut scored: Vec<(String, f64)> = blocks
        .iter()
        .map(|b| {
            let score = calculate_staleness(b, current_turn, config);
            (b.id.clone(), score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

    fn make_block(turn: u32, tokens: u32, refs: u32) -> Block {
        Block {
            id: format!("b{turn}"),
            role: Role::User,
            block_type: None,
            content: "test".to_string(),
            tokens,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "test".to_string(),
                    tokens,
                },
                trimmed: None,
                summarized: None,
                minimal: None,
            },
            usage_heat: 0.0,
            position_relevance: 0.0,
            last_referenced_turn: 0,
            reference_count: refs,
            topic_cluster: None,
            topic_keywords: vec![],
            metadata: BlockMetadata {
                provider: "test".to_string(),
                turn_index: turn,
                tool_name: None,
                file_paths: vec![],
            },
        }
    }

    #[test]
    fn test_newer_blocks_less_stale() {
        let config = StalenessConfig::default();
        let old_block = make_block(0, 500, 0);
        let new_block = make_block(9, 500, 0);

        let old_score = calculate_staleness(&old_block, 10, &config);
        let new_score = calculate_staleness(&new_block, 10, &config);

        assert!(old_score > new_score, "Old block should be more stale");
    }

    #[test]
    fn test_larger_blocks_more_stale() {
        let config = StalenessConfig::default();
        let small = make_block(0, 100, 0);
        let large = make_block(0, 2000, 0);

        let small_score = calculate_staleness(&small, 10, &config);
        let large_score = calculate_staleness(&large, 10, &config);

        assert!(
            large_score > small_score,
            "Larger block should be more stale"
        );
    }

    #[test]
    fn test_referenced_blocks_less_stale() {
        let config = StalenessConfig::default();
        let unreferenced = make_block(0, 500, 0);
        let referenced = make_block(0, 500, 5);

        let unreferenced_score = calculate_staleness(&unreferenced, 10, &config);
        let referenced_score = calculate_staleness(&referenced, 10, &config);

        assert!(
            unreferenced_score > referenced_score,
            "Referenced block should be less stale"
        );
    }

    #[test]
    fn test_current_turn_block_zero_staleness() {
        let config = StalenessConfig::default();
        let block = make_block(10, 500, 0);
        let score = calculate_staleness(&block, 10, &config);
        assert!(
            score.abs() < f64::EPSILON,
            "Current-turn block should have 0 staleness"
        );
    }

    #[test]
    fn test_ranking() {
        let config = StalenessConfig::default();
        let blocks = vec![
            make_block(0, 1000, 0), // old + large → most stale
            make_block(9, 100, 0),  // new + small → least stale
            make_block(5, 500, 2),  // middle + referenced
        ];

        let ranked = rank_by_staleness(&blocks, 10, &config);
        assert_eq!(ranked[0].0, "b0"); // most stale first
        assert!(ranked[0].1 > ranked[1].1);
    }
}
