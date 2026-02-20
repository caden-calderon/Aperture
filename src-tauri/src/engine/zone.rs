//! Zone auto-assignment and management.
//!
//! Blocks are assigned to zones based on their role and token proportions:
//! - System → Primacy (role-based)
//! - Pinned blocks → keep current zone
//! - Last ~15% of non-system tokens → Recency
//! - Everything else (including Thinking) → Middle
//!
//! Token proportions follow "lost in the middle" research:
//! Primacy ~10-15%, Middle ~70%, Recency ~15%.

use super::block::Block;
use super::types::{BuiltInZone, Role, Zone};

/// Configuration for token-proportional zone assignment.
#[derive(Debug, Clone)]
pub struct ZoneConfig {
    /// Fraction of non-system tokens to keep in recency (default 0.15 = 15%).
    pub recency_pct: f64,
    /// Don't split into middle/recency below this token count (default 10_000).
    pub middle_activation_tokens: u32,
    /// Always protect at least this many tokens in recency (default 2_000).
    pub min_recency_tokens: u32,
}

impl Default for ZoneConfig {
    fn default() -> Self {
        Self {
            recency_pct: 0.15,
            middle_activation_tokens: 10_000,
            min_recency_tokens: 2_000,
        }
    }
}

/// Assign zones to a batch of blocks using token-proportional logic.
///
/// Algorithm:
/// 1. System → Primacy (role-based)
/// 2. Pinned blocks keep their current zone
/// 3. If total non-system tokens < middle_activation_tokens → all non-system = Recency
/// 4. Otherwise: compute recency_budget from token proportions, walk blocks from
///    most-recent turn backward, accumulating tokens. Blocks within budget → Recency,
///    rest → Middle (including Thinking blocks).
pub fn assign_zones(blocks: &mut [Block], config: &ZoneConfig) {
    // Pass 1: Assign Primacy for system blocks, count non-system tokens
    let mut non_system_tokens: u64 = 0;
    for block in blocks.iter_mut() {
        if block.pinned.is_some() {
            // Pinned blocks keep their zone — don't count toward non-system budget
            continue;
        }
        if block.role == Role::System {
            block.zone = Zone::BuiltIn(BuiltInZone::Primacy);
            continue;
        }
        non_system_tokens += block.tokens as u64;
    }

    // Pass 2: If below activation threshold, all non-system/non-pinned → Recency
    if non_system_tokens < config.middle_activation_tokens as u64 {
        for block in blocks.iter_mut() {
            if block.pinned.is_some() {
                continue;
            }
            if block.role == Role::System {
                continue;
            }
            block.zone = Zone::BuiltIn(BuiltInZone::Recency);
        }
        return;
    }

    // Pass 3: Token-proportional split
    let recency_budget = (non_system_tokens as f64 * config.recency_pct)
        .max(config.min_recency_tokens as f64) as u64;

    // Collect non-system, non-pinned blocks with their indices, sorted by turn descending
    let mut assignable: Vec<(usize, u32, u32)> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.pinned.is_none() && b.role != Role::System)
        .map(|(i, b)| (i, b.metadata.turn_index, b.tokens))
        .collect();

    // Sort by turn_index descending (most recent first)
    assignable.sort_by(|a, b| b.1.cmp(&a.1));

    // Walk from most recent, accumulating tokens
    let mut accumulated: u64 = 0;
    let mut recency_indices = std::collections::HashSet::new();

    for &(idx, _turn, tokens) in &assignable {
        if accumulated + tokens as u64 <= recency_budget {
            accumulated += tokens as u64;
            recency_indices.insert(idx);
        } else {
            // If we haven't assigned anything yet and this is the most recent turn,
            // include it to satisfy min_recency_tokens guarantee
            if recency_indices.is_empty() {
                recency_indices.insert(idx);
            }
            break;
        }
    }

    // Apply zones
    for (i, block) in blocks.iter_mut().enumerate() {
        if block.pinned.is_some() {
            continue;
        }
        if block.role == Role::System {
            continue;
        }
        if recency_indices.contains(&i) {
            block.zone = Zone::BuiltIn(BuiltInZone::Recency);
        } else {
            block.zone = Zone::BuiltIn(BuiltInZone::Middle);
        }
    }
}

/// Context ordering weight for a zone (lower = earlier in context).
pub fn zone_context_order(zone: &Zone) -> u32 {
    match zone {
        Zone::BuiltIn(BuiltInZone::Primacy) => 0,
        Zone::BuiltIn(BuiltInZone::Middle) => 50,
        Zone::BuiltIn(BuiltInZone::Recency) => 1000,
        Zone::Custom(_) => 60, // between middle and recency
    }
}

/// Sort blocks into context order (primacy first, recency last).
/// Within each zone, preserves existing order.
pub fn sort_by_context_order(blocks: &mut [Block]) {
    blocks.sort_by(|a, b| {
        let za = zone_context_order(&a.zone);
        let zb = zone_context_order(&b.zone);
        za.cmp(&zb)
            .then_with(|| a.metadata.turn_index.cmp(&b.metadata.turn_index))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{CompressionLevel, PinPosition};

    fn make_block(id: &str, role: Role, turn_index: u32, tokens: u32) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: "test".to_string(),
            tokens,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Recency), // parser default
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
            reference_count: 0,
            topic_cluster: None,
            topic_keywords: vec![],
            metadata: BlockMetadata {
                provider: "test".to_string(),
                turn_index,
                tool_name: None,
                file_paths: vec![],
            },
        }
    }

    #[test]
    fn test_system_goes_to_primacy() {
        let config = ZoneConfig::default();
        let mut blocks = vec![make_block("b1", Role::System, 0, 500)];
        assign_zones(&mut blocks, &config);
        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy));
    }

    #[test]
    fn test_thinking_follows_token_proportional_rules_small_context() {
        // Below activation threshold: thinking goes to Recency like other non-system blocks
        let config = ZoneConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 500),
            make_block("think", Role::Thinking, 1, 200),
            make_block("u1", Role::User, 2, 1000),
        ];
        // Non-system: 1200 < 10_000 → all Recency
        assign_zones(&mut blocks, &config);
        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy)); // system
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Recency)); // thinking
        assert_eq!(blocks[2].zone, Zone::BuiltIn(BuiltInZone::Recency)); // user
    }

    #[test]
    fn test_thinking_follows_token_proportional_rules_large_context() {
        // Above activation: old thinking blocks go to Middle (archivable)
        let config = ZoneConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 1000),
            make_block("think1", Role::Thinking, 1, 3000),
            make_block("u1", Role::User, 2, 40_000),
            make_block("a1", Role::Assistant, 3, 40_000),
            make_block("think2", Role::Thinking, 4, 3000),
            make_block("u2", Role::User, 5, 5000),
        ];
        // Non-system: 91_000; recency budget = 13_650
        // Walk from turn 5: u2 (5k) → think2 (8k) → a1 (48k > 13_650)
        assign_zones(&mut blocks, &config);
        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy)); // system
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Middle)); // old thinking → archivable
        assert_eq!(blocks[2].zone, Zone::BuiltIn(BuiltInZone::Middle)); // old user
        assert_eq!(blocks[3].zone, Zone::BuiltIn(BuiltInZone::Middle)); // old assistant
        assert_eq!(blocks[4].zone, Zone::BuiltIn(BuiltInZone::Recency)); // recent thinking
        assert_eq!(blocks[5].zone, Zone::BuiltIn(BuiltInZone::Recency)); // recent user
    }

    #[test]
    fn test_small_context_all_recency() {
        // Total non-system tokens < middle_activation_tokens (10k) → all Recency
        let config = ZoneConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 500),
            make_block("u1", Role::User, 1, 1000),
            make_block("a1", Role::Assistant, 2, 2000),
            make_block("u2", Role::User, 3, 1000),
        ];
        // Non-system total: 4000 < 10_000
        assign_zones(&mut blocks, &config);

        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy));
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Recency));
        assert_eq!(blocks[2].zone, Zone::BuiltIn(BuiltInZone::Recency));
        assert_eq!(blocks[3].zone, Zone::BuiltIn(BuiltInZone::Recency));
    }

    #[test]
    fn test_large_context_splits_middle_recency() {
        // 100k+ total tokens → ~15% in Recency, rest in Middle
        let config = ZoneConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 2000),
            make_block("u1", Role::User, 1, 20_000),
            make_block("a1", Role::Assistant, 2, 30_000),
            make_block("u2", Role::User, 3, 25_000),
            make_block("a2", Role::Assistant, 4, 20_000),
            make_block("u3", Role::User, 5, 5_000),
        ];
        // Non-system total: 100_000; recency budget = 15_000
        // Walk from turn 5 backward: u3 (5000) → a2 (20_000 → 25_000 > 15_000)
        // So u3 fits, a2 does not
        assign_zones(&mut blocks, &config);

        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy)); // system
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Middle)); // turn 1
        assert_eq!(blocks[2].zone, Zone::BuiltIn(BuiltInZone::Middle)); // turn 2
        assert_eq!(blocks[3].zone, Zone::BuiltIn(BuiltInZone::Middle)); // turn 3
        assert_eq!(blocks[4].zone, Zone::BuiltIn(BuiltInZone::Middle)); // turn 4
        assert_eq!(blocks[5].zone, Zone::BuiltIn(BuiltInZone::Recency)); // turn 5, fits in budget
    }

    #[test]
    fn test_huge_single_block_goes_to_middle_min_recency_protects_latest() {
        // One giant block + one small latest turn
        let config = ZoneConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 1000),
            make_block("giant", Role::Assistant, 1, 80_000),
            make_block("latest", Role::User, 2, 500),
        ];
        // Non-system: 80_500; recency budget = max(80_500 * 0.15, 2000) = 12_075
        // Walk from turn 2: latest (500) fits → turn 1: giant (80_000 → 80_500 > 12_075)
        assign_zones(&mut blocks, &config);

        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy));
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Middle));
        assert_eq!(blocks[2].zone, Zone::BuiltIn(BuiltInZone::Recency));
    }

    #[test]
    fn test_pinned_blocks_keep_zone() {
        let config = ZoneConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 1000),
            make_block("pinned", Role::User, 1, 20_000),
            make_block("u2", Role::User, 2, 50_000),
            make_block("u3", Role::User, 3, 30_000),
        ];
        blocks[1].pinned = Some(PinPosition::Top);
        blocks[1].zone = Zone::BuiltIn(BuiltInZone::Primacy);

        assign_zones(&mut blocks, &config);

        // Pinned block keeps its zone regardless of token proportions
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Primacy));
    }

    #[test]
    fn test_empty_blocks_no_crash() {
        let config = ZoneConfig::default();
        let mut blocks: Vec<Block> = vec![];
        assign_zones(&mut blocks, &config);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_single_non_system_block() {
        let config = ZoneConfig::default();
        let mut blocks = vec![make_block("u1", Role::User, 0, 500)];
        // 500 < 10_000 → all recency
        assign_zones(&mut blocks, &config);
        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Recency));
    }

    #[test]
    fn test_context_order_sorting() {
        let mut blocks = vec![
            make_block("rec", Role::User, 9, 100),
            make_block("mid", Role::User, 2, 100),
            make_block("sys", Role::System, 0, 100),
        ];
        blocks[0].zone = Zone::BuiltIn(BuiltInZone::Recency);
        blocks[1].zone = Zone::BuiltIn(BuiltInZone::Middle);
        blocks[2].zone = Zone::BuiltIn(BuiltInZone::Primacy);

        sort_by_context_order(&mut blocks);

        assert_eq!(blocks[0].id, "sys"); // primacy first
        assert_eq!(blocks[1].id, "mid"); // middle second
        assert_eq!(blocks[2].id, "rec"); // recency last
    }

    #[test]
    fn test_multiple_recent_turns_fit_in_recency_budget() {
        let config = ZoneConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 1000),
            make_block("u1", Role::User, 1, 40_000),
            make_block("a1", Role::Assistant, 2, 40_000),
            make_block("u2", Role::User, 3, 5_000),
            make_block("a2", Role::Assistant, 4, 5_000),
            make_block("u3", Role::User, 5, 5_000),
            make_block("a3", Role::Assistant, 6, 5_000),
        ];
        // Non-system: 100_000; recency budget = 15_000
        // Walk from turn 6: a3 (5k) → u3 (10k) → a2 (15k) → u2 (20k > 15k)
        assign_zones(&mut blocks, &config);

        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy));
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Middle));
        assert_eq!(blocks[2].zone, Zone::BuiltIn(BuiltInZone::Middle));
        assert_eq!(blocks[3].zone, Zone::BuiltIn(BuiltInZone::Middle)); // u2 doesn't fit
        assert_eq!(blocks[4].zone, Zone::BuiltIn(BuiltInZone::Recency)); // a2 fits
        assert_eq!(blocks[5].zone, Zone::BuiltIn(BuiltInZone::Recency)); // u3 fits
        assert_eq!(blocks[6].zone, Zone::BuiltIn(BuiltInZone::Recency)); // a3 fits
    }

    #[test]
    fn test_min_recency_tokens_guarantees_latest_turn() {
        // Even if recency_pct yields a tiny budget, min_recency_tokens ensures coverage
        let config = ZoneConfig {
            recency_pct: 0.01, // very small
            middle_activation_tokens: 5_000,
            min_recency_tokens: 3_000,
        };
        let mut blocks = vec![
            make_block("u1", Role::User, 1, 10_000),
            make_block("u2", Role::User, 2, 2_500),
        ];
        // Non-system: 12_500; recency_pct budget = 125, min = 3_000 → budget = 3_000
        // Walk from turn 2: u2 (2_500) fits
        assign_zones(&mut blocks, &config);

        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Middle));
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Recency));
    }
}
