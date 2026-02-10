//! Zone auto-assignment and management.
//!
//! Blocks are assigned to zones based on their role and position:
//! - System prompts → Primacy
//! - Last N turns → Recency
//! - Everything else → Middle
//! - Manual pins override auto-assignment

use super::block::Block;
use super::types::{BuiltInZone, Role, Zone};

/// Configuration for zone auto-assignment.
#[derive(Debug, Clone)]
pub struct ZoneConfig {
    /// Number of recent turns to keep in the recency zone.
    pub recency_window: u32,
    /// Keep all non-system blocks in recency until this many turns are present.
    pub middle_activation_turns: u32,
}

impl Default for ZoneConfig {
    fn default() -> Self {
        Self {
            recency_window: 5,
            middle_activation_turns: 8,
        }
    }
}

/// Automatically assign a zone to a block based on its properties.
///
/// Rules (in priority order):
/// 1. If block is pinned, its zone is not changed (pin overrides)
/// 2. System/thinking role → Primacy
/// 3. Before middle activation threshold, all non-system blocks → Recency
/// 4. Turn index within recency window → Recency
/// 5. Everything else → Middle
pub fn auto_assign_zone(block: &Block, total_turns: u32, config: &ZoneConfig) -> Zone {
    // Pinned blocks keep their current zone
    if block.pinned.is_some() {
        return block.zone.clone();
    }

    // System prompts and thinking blocks → primacy
    if block.role == Role::System || block.role == Role::Thinking {
        return Zone::BuiltIn(BuiltInZone::Primacy);
    }

    // Keep simple two-zone behavior for smaller conversations.
    if total_turns <= config.middle_activation_turns {
        return Zone::BuiltIn(BuiltInZone::Recency);
    }

    // Recent turns → recency
    if total_turns > 0
        && block.metadata.turn_index >= total_turns.saturating_sub(config.recency_window)
    {
        return Zone::BuiltIn(BuiltInZone::Recency);
    }

    // Default → middle
    Zone::BuiltIn(BuiltInZone::Middle)
}

/// Assign zones to a batch of blocks.
pub fn assign_zones(blocks: &mut [Block], config: &ZoneConfig) {
    let max_turn = blocks
        .iter()
        .map(|b| b.metadata.turn_index)
        .max()
        .unwrap_or(0);
    let total_turns = max_turn + 1;

    for block in blocks.iter_mut() {
        let zone = auto_assign_zone(block, total_turns, config);
        block.zone = zone;
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

    fn make_block(id: &str, role: Role, turn_index: u32) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: "test".to_string(),
            tokens: 10,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Recency), // parser default
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "test".to_string(),
                    tokens: 10,
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
        let block = make_block("b1", Role::System, 0);
        let config = ZoneConfig::default();
        let zone = auto_assign_zone(&block, 10, &config);
        assert_eq!(zone, Zone::BuiltIn(BuiltInZone::Primacy));
    }

    #[test]
    fn test_thinking_goes_to_primacy() {
        let block = make_block("b1", Role::Thinking, 3);
        let config = ZoneConfig::default();
        let zone = auto_assign_zone(&block, 10, &config);
        assert_eq!(zone, Zone::BuiltIn(BuiltInZone::Primacy));
    }

    #[test]
    fn test_recent_turns_go_to_recency() {
        let config = ZoneConfig {
            recency_window: 3,
            ..ZoneConfig::default()
        };
        // Total turns = 10, recency window = 3 → turns 7,8,9 go to recency
        let block = make_block("b1", Role::User, 8);
        let zone = auto_assign_zone(&block, 10, &config);
        assert_eq!(zone, Zone::BuiltIn(BuiltInZone::Recency));
    }

    #[test]
    fn test_old_turns_go_to_middle() {
        let config = ZoneConfig {
            recency_window: 3,
            middle_activation_turns: 4,
        };
        let block = make_block("b1", Role::User, 2);
        let zone = auto_assign_zone(&block, 10, &config);
        assert_eq!(zone, Zone::BuiltIn(BuiltInZone::Middle));
    }

    #[test]
    fn test_pinned_blocks_keep_zone() {
        let mut block = make_block("b1", Role::User, 2);
        block.pinned = Some(PinPosition::Top);
        block.zone = Zone::BuiltIn(BuiltInZone::Primacy);

        let config = ZoneConfig::default();
        let zone = auto_assign_zone(&block, 10, &config);
        // Should keep primacy because of pin
        assert_eq!(zone, Zone::BuiltIn(BuiltInZone::Primacy));
    }

    #[test]
    fn test_assign_zones_batch() {
        let config = ZoneConfig {
            recency_window: 2,
            middle_activation_turns: 4,
        };
        let mut blocks = vec![
            make_block("sys", Role::System, 0),
            make_block("u1", Role::User, 1),
            make_block("a1", Role::Assistant, 2),
            make_block("u2", Role::User, 3),
            make_block("a2", Role::Assistant, 4),
        ];

        assign_zones(&mut blocks, &config);

        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy)); // system
        assert_eq!(blocks[1].zone, Zone::BuiltIn(BuiltInZone::Middle)); // turn 1
        assert_eq!(blocks[2].zone, Zone::BuiltIn(BuiltInZone::Middle)); // turn 2
        assert_eq!(blocks[3].zone, Zone::BuiltIn(BuiltInZone::Recency)); // turn 3 (in window)
        assert_eq!(blocks[4].zone, Zone::BuiltIn(BuiltInZone::Recency)); // turn 4 (in window)
    }

    #[test]
    fn test_before_middle_activation_all_non_system_are_recency() {
        let config = ZoneConfig {
            recency_window: 2,
            middle_activation_turns: 8,
        };
        let block = make_block("b1", Role::Assistant, 1);
        let zone = auto_assign_zone(&block, 4, &config);
        assert_eq!(zone, Zone::BuiltIn(BuiltInZone::Recency));
    }

    #[test]
    fn test_context_order_sorting() {
        let mut blocks = vec![
            make_block("rec", Role::User, 9),
            make_block("mid", Role::User, 2),
            make_block("sys", Role::System, 0),
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
    fn test_all_blocks_in_recency_with_small_window() {
        let config = ZoneConfig {
            recency_window: 10,
            ..ZoneConfig::default()
        };
        // Only 3 turns total, window=10 → all non-system go to recency
        let block = make_block("b1", Role::User, 1);
        let zone = auto_assign_zone(&block, 3, &config);
        assert_eq!(zone, Zone::BuiltIn(BuiltInZone::Recency));
    }
}
