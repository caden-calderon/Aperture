//! Mutation applicator — translates planner mutations into JSON rewrite decisions.
//!
//! The applicator bridges between the planner's high-level `ContextMutation` enum
//! and the concrete JSON-level operations that `rewriter.rs` applies to payloads.
//! It works at the block/turn level, mapping block IDs to turn indices and deciding
//! which turns to remove, which content to replace, and which engine-side updates
//! to make.

use std::collections::{HashMap, HashSet};

use crate::engine::block::Block;
use crate::engine::planner::types::ContextMutation;
use crate::engine::types::{PinPosition, Zone};

/// An engine-side block update (zone shift, pin toggle, etc.).
/// These don't affect the JSON payload but update the engine's internal state.
#[derive(Debug, Clone)]
pub struct EngineBlockUpdate {
    pub block_id: String,
    pub kind: EngineUpdateKind,
}

/// The kind of engine-side update to apply.
#[derive(Debug, Clone)]
pub enum EngineUpdateKind {
    SetZone(Zone),
    SetPinned(Option<PinPosition>),
    Archive,
    ApplyCompression { summary: String },
    RestoreOriginal,
    UpdateContent { new_content: String },
}

/// Concrete decisions for how to rewrite the JSON payload.
#[derive(Debug, Clone, Default)]
pub struct RewriteDecisions {
    /// Turn indices to remove entirely from the payload (archived blocks).
    pub remove_turns: HashSet<u32>,
    /// Turn index → new content (compressed or file-updated blocks).
    pub content_replacements: HashMap<u32, String>,
    /// Blocks modified in the engine (zone shifts, pins, etc.).
    pub engine_updates: Vec<EngineBlockUpdate>,
}

impl RewriteDecisions {
    /// Whether any payload-level changes are needed.
    pub fn has_payload_changes(&self) -> bool {
        !self.remove_turns.is_empty() || !self.content_replacements.is_empty()
    }
}

/// Apply a set of mutations to produce rewrite decisions.
///
/// When archiving, if multiple blocks share a `turn_index`, the turn is only
/// removed if ALL blocks at that index are archived. Otherwise, partial
/// archival is noted but the turn is kept (content-level removal deferred).
pub fn apply_mutations(blocks: &[Block], mutations: &[ContextMutation]) -> RewriteDecisions {
    let mut decisions = RewriteDecisions::default();

    // Build lookup maps
    let block_by_id: HashMap<&str, &Block> = blocks.iter().map(|b| (b.id.as_str(), b)).collect();

    // Track which block IDs are being archived (for partial-turn detection)
    let mut archived_block_ids: HashSet<&str> = HashSet::new();

    for mutation in mutations {
        match mutation {
            ContextMutation::Archive { block_id } => {
                if block_by_id.contains_key(block_id.as_str()) {
                    archived_block_ids.insert(block_id.as_str());
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: block_id.clone(),
                        kind: EngineUpdateKind::Archive,
                    });
                }
            }
            ContextMutation::Compress {
                block_id, summary, ..
            } => {
                if let Some(block) = block_by_id.get(block_id.as_str()) {
                    decisions
                        .content_replacements
                        .insert(block.metadata.turn_index, summary.clone());
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: block_id.clone(),
                        kind: EngineUpdateKind::ApplyCompression {
                            summary: summary.clone(),
                        },
                    });
                }
            }
            ContextMutation::UpdateContent {
                block_id,
                new_content,
            } => {
                if let Some(block) = block_by_id.get(block_id.as_str()) {
                    decisions
                        .content_replacements
                        .insert(block.metadata.turn_index, new_content.clone());
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: block_id.clone(),
                        kind: EngineUpdateKind::UpdateContent {
                            new_content: new_content.clone(),
                        },
                    });
                }
            }
            ContextMutation::Expand { block_id } => {
                if let Some(block) = block_by_id.get(block_id.as_str()) {
                    // Restore original content
                    let original = &block.compressed_versions.original.content;
                    decisions
                        .content_replacements
                        .insert(block.metadata.turn_index, original.clone());
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: block_id.clone(),
                        kind: EngineUpdateKind::RestoreOriginal,
                    });
                }
            }
            ContextMutation::Shift {
                block_id,
                target_zone,
            } => {
                decisions.engine_updates.push(EngineBlockUpdate {
                    block_id: block_id.clone(),
                    kind: EngineUpdateKind::SetZone(Zone::BuiltIn(*target_zone)),
                });
            }
            ContextMutation::Pin { block_id } => {
                decisions.engine_updates.push(EngineBlockUpdate {
                    block_id: block_id.clone(),
                    kind: EngineUpdateKind::SetPinned(Some(PinPosition::Top)),
                });
            }
            ContextMutation::Unpin { block_id } => {
                decisions.engine_updates.push(EngineBlockUpdate {
                    block_id: block_id.clone(),
                    kind: EngineUpdateKind::SetPinned(None),
                });
            }
            ContextMutation::Recall { .. } => {
                // Recall from archive is engine-side only; the block isn't in
                // the current payload so there's nothing to rewrite.
            }
            ContextMutation::Split { .. } => {
                // Deferred to Phase 4 — no-op in v1.
            }
        }
    }

    // Resolve archived turns: only remove a turn if ALL blocks at that index are archived
    if !archived_block_ids.is_empty() {
        // Group blocks by turn_index
        let mut blocks_per_turn: HashMap<u32, Vec<&str>> = HashMap::new();
        for block in blocks {
            blocks_per_turn
                .entry(block.metadata.turn_index)
                .or_default()
                .push(block.id.as_str());
        }

        for (turn_index, block_ids_at_turn) in &blocks_per_turn {
            let all_archived = block_ids_at_turn
                .iter()
                .all(|id| archived_block_ids.contains(id));
            if all_archived
                && block_ids_at_turn
                    .iter()
                    .any(|id| archived_block_ids.contains(id))
            {
                decisions.remove_turns.insert(*turn_index);
            }
        }
    }

    // Content replacements should not target turns that are being removed
    decisions.remove_turns.iter().for_each(|t| {
        decisions.content_replacements.remove(t);
    });

    // Archival dominates other updates for the same block.
    if !archived_block_ids.is_empty() {
        decisions.engine_updates.retain(|update| {
            if !archived_block_ids.contains(update.block_id.as_str()) {
                return true;
            }
            matches!(update.kind, EngineUpdateKind::Archive)
        });
    }

    decisions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

    fn mock_block(id: &str, turn_index: u32) -> Block {
        Block {
            id: id.to_string(),
            role: Role::User,
            block_type: None,
            content: format!("Content of block {id}"),
            tokens: 100,
            timestamp: "2026-02-13T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: format!("Original content of block {id}"),
                    tokens: 100,
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
    fn test_archive_adds_turn_to_remove() {
        let blocks = vec![mock_block("b1", 3)];
        let mutations = vec![ContextMutation::Archive {
            block_id: "b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.contains(&3));
        assert!(decisions.content_replacements.is_empty());
        assert_eq!(decisions.engine_updates.len(), 1);
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::Archive
        ));
    }

    #[test]
    fn test_compress_adds_content_replacement() {
        let blocks = vec![mock_block("b1", 2)];
        let mutations = vec![ContextMutation::Compress {
            block_id: "b1".into(),
            summary: "Compressed summary".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
        assert_eq!(
            decisions.content_replacements.get(&2),
            Some(&"Compressed summary".to_string())
        );
        assert_eq!(decisions.engine_updates.len(), 1);
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::ApplyCompression { .. }
        ));
    }

    #[test]
    fn test_update_content_adds_content_replacement() {
        let blocks = vec![mock_block("b1", 5)];
        let mutations = vec![ContextMutation::UpdateContent {
            block_id: "b1".into(),
            new_content: "Updated file content".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert_eq!(
            decisions.content_replacements.get(&5),
            Some(&"Updated file content".to_string())
        );
        assert_eq!(decisions.engine_updates.len(), 1);
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::UpdateContent { .. }
        ));
    }

    #[test]
    fn test_expand_restores_original_content() {
        let blocks = vec![mock_block("b1", 4)];
        let mutations = vec![ContextMutation::Expand {
            block_id: "b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert_eq!(
            decisions.content_replacements.get(&4),
            Some(&"Original content of block b1".to_string())
        );
        assert_eq!(decisions.engine_updates.len(), 1);
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::RestoreOriginal
        ));
    }

    #[test]
    fn test_shift_produces_engine_update_only() {
        let blocks = vec![mock_block("b1", 1)];
        let mutations = vec![ContextMutation::Shift {
            block_id: "b1".into(),
            target_zone: BuiltInZone::Primacy,
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
        assert!(decisions.content_replacements.is_empty());
        assert_eq!(decisions.engine_updates.len(), 1);
        assert_eq!(decisions.engine_updates[0].block_id, "b1");
    }

    #[test]
    fn test_pin_produces_engine_update_only() {
        let blocks = vec![mock_block("b1", 1)];
        let mutations = vec![ContextMutation::Pin {
            block_id: "b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
        assert!(decisions.content_replacements.is_empty());
        assert_eq!(decisions.engine_updates.len(), 1);
    }

    #[test]
    fn test_unpin_produces_engine_update_only() {
        let blocks = vec![mock_block("b1", 1)];
        let mutations = vec![ContextMutation::Unpin {
            block_id: "b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
        assert!(decisions.content_replacements.is_empty());
        assert_eq!(decisions.engine_updates.len(), 1);
    }

    #[test]
    fn test_unknown_block_id_gracefully_skipped() {
        let blocks = vec![mock_block("b1", 1)];
        let mutations = vec![ContextMutation::Archive {
            block_id: "nonexistent".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
    }

    #[test]
    fn test_partial_archival_turn_not_removed() {
        // Two blocks at same turn_index, only one archived
        let mut b1 = mock_block("b1", 3);
        b1.role = Role::Assistant;
        let mut b2 = mock_block("b2", 3);
        b2.role = Role::ToolUse;

        let blocks = vec![b1, b2];
        let mutations = vec![ContextMutation::Archive {
            block_id: "b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &mutations);
        // Turn should NOT be removed because b2 is still active
        assert!(!decisions.remove_turns.contains(&3));
        assert_eq!(decisions.engine_updates.len(), 1);
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::Archive
        ));
    }

    #[test]
    fn test_all_blocks_at_turn_archived_removes_turn() {
        let mut b1 = mock_block("b1", 3);
        b1.role = Role::Assistant;
        let mut b2 = mock_block("b2", 3);
        b2.role = Role::ToolUse;

        let blocks = vec![b1, b2];
        let mutations = vec![
            ContextMutation::Archive {
                block_id: "b1".into(),
            },
            ContextMutation::Archive {
                block_id: "b2".into(),
            },
        ];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.contains(&3));
        assert_eq!(decisions.engine_updates.len(), 2);
        assert!(decisions
            .engine_updates
            .iter()
            .all(|u| matches!(u.kind, EngineUpdateKind::Archive)));
    }

    #[test]
    fn test_archive_dominates_other_updates_for_same_block() {
        let blocks = vec![mock_block("b1", 7)];
        let mutations = vec![
            ContextMutation::Compress {
                block_id: "b1".into(),
                summary: "Compressed".into(),
            },
            ContextMutation::Archive {
                block_id: "b1".into(),
            },
            ContextMutation::UpdateContent {
                block_id: "b1".into(),
                new_content: "Updated".into(),
            },
        ];

        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.contains(&7));
        assert_eq!(decisions.engine_updates.len(), 1);
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::Archive
        ));
    }

    #[test]
    fn test_multiple_mutations_on_same_block_last_wins() {
        let blocks = vec![mock_block("b1", 2)];
        let mutations = vec![
            ContextMutation::Compress {
                block_id: "b1".into(),
                summary: "First summary".into(),
            },
            ContextMutation::Compress {
                block_id: "b1".into(),
                summary: "Second summary".into(),
            },
        ];
        let decisions = apply_mutations(&blocks, &mutations);
        // Last write wins for content_replacements (HashMap insert)
        assert_eq!(
            decisions.content_replacements.get(&2),
            Some(&"Second summary".to_string())
        );
    }

    #[test]
    fn test_recall_and_split_are_noop() {
        let blocks = vec![mock_block("b1", 1)];
        let mutations = vec![
            ContextMutation::Recall {
                block_id: "archived_1".into(),
            },
            ContextMutation::Split {
                thread_id: "t1".into(),
                at_turn: 5,
                archive_before: true,
            },
        ];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
        assert!(decisions.content_replacements.is_empty());
        assert!(decisions.engine_updates.is_empty());
    }

    #[test]
    fn test_empty_mutations_produces_no_changes() {
        let blocks = vec![mock_block("b1", 1), mock_block("b2", 2)];
        let decisions = apply_mutations(&blocks, &[]);
        assert!(!decisions.has_payload_changes());
    }

    #[test]
    fn test_archive_and_compress_different_turns() {
        let blocks = vec![mock_block("b1", 1), mock_block("b2", 3)];
        let mutations = vec![
            ContextMutation::Archive {
                block_id: "b1".into(),
            },
            ContextMutation::Compress {
                block_id: "b2".into(),
                summary: "Summary".into(),
            },
        ];
        let decisions = apply_mutations(&blocks, &mutations);
        assert!(decisions.remove_turns.contains(&1));
        assert_eq!(
            decisions.content_replacements.get(&3),
            Some(&"Summary".to_string())
        );
    }
}
