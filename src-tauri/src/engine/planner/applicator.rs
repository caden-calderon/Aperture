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

/// A content-block-level stub for partial-turn archival.
///
/// When not all blocks at a turn are archived, we can't remove the entire turn.
/// Instead, we replace individual content blocks with short stubs.
#[derive(Debug, Clone)]
pub struct PartialTurnStub {
    /// Turn index of the parent message.
    pub turn_index: u32,
    /// Index of the content block within the message's content array.
    pub content_index: usize,
    /// Replacement stub text (e.g. `[archived: 3,500 tokens]`).
    pub stub: String,
}

/// Concrete decisions for how to rewrite the JSON payload.
#[derive(Debug, Clone, Default)]
pub struct RewriteDecisions {
    /// Turn indices to remove entirely from the payload (archived blocks).
    pub remove_turns: HashSet<u32>,
    /// Turn index → new content (compressed or file-updated blocks).
    pub content_replacements: HashMap<u32, String>,
    /// Content-block-level stubs for partial-turn archival.
    pub partial_turn_stubs: Vec<PartialTurnStub>,
    /// Blocks modified in the engine (zone shifts, pins, etc.).
    pub engine_updates: Vec<EngineBlockUpdate>,
}

impl RewriteDecisions {
    /// Whether any payload-level changes are needed.
    pub fn has_payload_changes(&self) -> bool {
        !self.remove_turns.is_empty()
            || !self.content_replacements.is_empty()
            || !self.partial_turn_stubs.is_empty()
    }
}

/// Strip a leading `#` from a block ID (defense-in-depth).
fn normalize_block_id(id: &str) -> &str {
    id.strip_prefix('#').unwrap_or(id)
}

/// Format an archival stub for a content block.
fn format_archive_stub(tokens: u32) -> String {
    if tokens >= 1000 {
        let k = tokens as f64 / 1000.0;
        if k >= 10.0 {
            format!("[archived: {}k tokens]", k.round() as u64)
        } else {
            format!("[archived: {k:.1}k tokens]")
        }
    } else {
        format!("[archived: {tokens} tokens]")
    }
}

/// Apply a set of mutations to produce rewrite decisions.
///
/// Two block sources are used:
/// - `engine_blocks`: blocks cached in the engine (enriched metadata, compressed versions).
///   Used for Expand (original content) and engine-side updates.
/// - `request_blocks`: blocks parsed from the CURRENT request body (pre-rewrite).
///   Includes content that was previously archived but re-sent by stateless clients.
///   Used for Archive turn_index mapping and turn grouping.
///
/// When archiving, if multiple blocks share a `turn_index`, the turn is only
/// removed if ALL blocks at that index are archived. Otherwise, partial
/// archival is noted but the turn is kept (content-level removal deferred).
pub fn apply_mutations(
    engine_blocks: &[Block],
    request_blocks: &[Block],
    mutations: &[ContextMutation],
) -> RewriteDecisions {
    let mut decisions = RewriteDecisions::default();

    // Build dual lookup maps.
    // engine_by_id: enriched blocks with compressed_versions, metadata, etc.
    // request_by_id: blocks from current request — includes archived content re-sent by client.
    let engine_by_id: HashMap<&str, &Block> =
        engine_blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    let request_by_id: HashMap<&str, &Block> =
        request_blocks.iter().map(|b| (b.id.as_str(), b)).collect();

    // Track which block IDs are being archived (for partial-turn detection)
    let mut archived_block_ids: HashSet<&str> = HashSet::new();

    for mutation in mutations {
        match mutation {
            ContextMutation::Archive { block_id } => {
                let nid = normalize_block_id(block_id);
                // Check request_by_id: archived blocks may not be in engine but ARE
                // in the current request (re-sent by stateless client).
                if request_by_id.contains_key(nid) || engine_by_id.contains_key(nid) {
                    archived_block_ids.insert(nid);
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: nid.to_string(),
                        kind: EngineUpdateKind::Archive,
                    });
                }
            }
            ContextMutation::Compress {
                block_id, summary, ..
            } => {
                let nid = normalize_block_id(block_id);
                // Use request_by_id for turn_index (matches current JSON payload)
                let turn_source = request_by_id.get(nid).or_else(|| engine_by_id.get(nid));
                if let Some(block) = turn_source {
                    decisions
                        .content_replacements
                        .insert(block.metadata.turn_index, summary.clone());
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: nid.to_string(),
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
                let nid = normalize_block_id(block_id);
                let turn_source = request_by_id.get(nid).or_else(|| engine_by_id.get(nid));
                if let Some(block) = turn_source {
                    decisions
                        .content_replacements
                        .insert(block.metadata.turn_index, new_content.clone());
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: nid.to_string(),
                        kind: EngineUpdateKind::UpdateContent {
                            new_content: new_content.clone(),
                        },
                    });
                }
            }
            ContextMutation::Expand { block_id } => {
                let nid = normalize_block_id(block_id);
                // Expand needs engine_by_id for original content (compressed_versions),
                // but request_by_id for turn_index (current payload position).
                if let Some(engine_block) = engine_by_id.get(nid) {
                    let original = &engine_block.compressed_versions.original.content;
                    let turn_index = request_by_id
                        .get(nid)
                        .map(|b| b.metadata.turn_index)
                        .unwrap_or(engine_block.metadata.turn_index);
                    decisions
                        .content_replacements
                        .insert(turn_index, original.clone());
                    decisions.engine_updates.push(EngineBlockUpdate {
                        block_id: nid.to_string(),
                        kind: EngineUpdateKind::RestoreOriginal,
                    });
                }
            }
            ContextMutation::Shift {
                block_id,
                target_zone,
            } => {
                let nid = normalize_block_id(block_id);
                decisions.engine_updates.push(EngineBlockUpdate {
                    block_id: nid.to_string(),
                    kind: EngineUpdateKind::SetZone(Zone::BuiltIn(*target_zone)),
                });
            }
            ContextMutation::Pin { block_id } => {
                let nid = normalize_block_id(block_id);
                decisions.engine_updates.push(EngineBlockUpdate {
                    block_id: nid.to_string(),
                    kind: EngineUpdateKind::SetPinned(Some(PinPosition::Top)),
                });
            }
            ContextMutation::Unpin { block_id } => {
                let nid = normalize_block_id(block_id);
                decisions.engine_updates.push(EngineBlockUpdate {
                    block_id: nid.to_string(),
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

    // Resolve archived turns: remove the full turn if ALL blocks are archived,
    // or generate per-block content stubs for partial-turn archival.
    // Use request_blocks for turn grouping — they reflect the current JSON payload layout,
    // including blocks that were previously archived but re-sent by the stateless client.
    if !archived_block_ids.is_empty() {
        // Preserve insertion order per turn (matches content array position).
        let mut blocks_per_turn: Vec<(u32, Vec<&Block>)> = Vec::new();
        let mut turn_index_map: HashMap<u32, usize> = HashMap::new();
        for block in request_blocks {
            if let Some(&vec_idx) = turn_index_map.get(&block.metadata.turn_index) {
                blocks_per_turn[vec_idx].1.push(block);
            } else {
                let vec_idx = blocks_per_turn.len();
                turn_index_map.insert(block.metadata.turn_index, vec_idx);
                blocks_per_turn.push((block.metadata.turn_index, vec![block]));
            }
        }
        // Fall back to engine_blocks for any blocks not in request_blocks
        // (shouldn't happen normally, but defense in depth)
        if blocks_per_turn.is_empty() {
            for block in engine_blocks {
                if let Some(&vec_idx) = turn_index_map.get(&block.metadata.turn_index) {
                    blocks_per_turn[vec_idx].1.push(block);
                } else {
                    let vec_idx = blocks_per_turn.len();
                    turn_index_map.insert(block.metadata.turn_index, vec_idx);
                    blocks_per_turn.push((block.metadata.turn_index, vec![block]));
                }
            }
        }

        for (turn_index, blocks_at_turn) in &blocks_per_turn {
            let archived_count = blocks_at_turn
                .iter()
                .filter(|b| archived_block_ids.contains(b.id.as_str()))
                .count();

            if archived_count == 0 {
                continue;
            }

            if archived_count == blocks_at_turn.len() {
                // All blocks archived → remove entire turn
                decisions.remove_turns.insert(*turn_index);
            } else {
                // Partial turn → generate content stubs for archived blocks
                for (content_index, block) in blocks_at_turn.iter().enumerate() {
                    if archived_block_ids.contains(block.id.as_str()) {
                        decisions.partial_turn_stubs.push(PartialTurnStub {
                            turn_index: *turn_index,
                            content_index,
                            stub: format_archive_stub(block.tokens),
                        });
                    }
                }
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
    }

    #[test]
    fn test_partial_archival_turn_not_removed_but_stubs_generated() {
        // Two blocks at same turn_index, only one archived
        let mut b1 = mock_block("b1", 3);
        b1.role = Role::Assistant;
        let mut b2 = mock_block("b2", 3);
        b2.role = Role::ToolUse;

        let blocks = vec![b1, b2];
        let mutations = vec![ContextMutation::Archive {
            block_id: "b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
        // Turn should NOT be removed because b2 is still active
        assert!(!decisions.remove_turns.contains(&3));
        // But a content stub SHOULD be generated for b1
        assert!(
            decisions.has_payload_changes(),
            "Partial-turn archival must produce payload changes via stubs"
        );
        assert_eq!(decisions.partial_turn_stubs.len(), 1);
        assert_eq!(decisions.partial_turn_stubs[0].turn_index, 3);
        assert_eq!(decisions.partial_turn_stubs[0].content_index, 0); // b1 is first block
        assert!(decisions.partial_turn_stubs[0].stub.contains("archived"));
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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

        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
        assert!(decisions.remove_turns.is_empty());
        assert!(decisions.content_replacements.is_empty());
        assert!(decisions.engine_updates.is_empty());
    }

    #[test]
    fn test_empty_mutations_produces_no_changes() {
        let blocks = vec![mock_block("b1", 1), mock_block("b2", 2)];
        let decisions = apply_mutations(&blocks, &blocks, &[]);
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
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
        assert!(decisions.remove_turns.contains(&1));
        assert_eq!(
            decisions.content_replacements.get(&3),
            Some(&"Summary".to_string())
        );
    }

    #[test]
    fn test_apply_mutations_hash_prefixed_archive() {
        let blocks = vec![mock_block("b1", 3)];
        let mutations = vec![ContextMutation::Archive {
            block_id: "#b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
        assert!(decisions.remove_turns.contains(&3));
        assert_eq!(decisions.engine_updates.len(), 1);
        // Engine update should use bare ID
        assert_eq!(decisions.engine_updates[0].block_id, "b1");
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::Archive
        ));
    }

    #[test]
    fn test_apply_mutations_hash_prefixed_compress() {
        let blocks = vec![mock_block("b1", 2)];
        let mutations = vec![ContextMutation::Compress {
            block_id: "#b1".into(),
            summary: "Short summary".into(),
        }];
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
        assert_eq!(
            decisions.content_replacements.get(&2),
            Some(&"Short summary".to_string())
        );
        assert_eq!(decisions.engine_updates[0].block_id, "b1");
    }

    #[test]
    fn test_apply_mutations_hash_prefixed_pin() {
        let blocks = vec![mock_block("b1", 1)];
        let mutations = vec![ContextMutation::Pin {
            block_id: "#b1".into(),
        }];
        let decisions = apply_mutations(&blocks, &blocks, &mutations);
        assert_eq!(decisions.engine_updates.len(), 1);
        assert_eq!(decisions.engine_updates[0].block_id, "b1");
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::SetPinned(Some(PinPosition::Top))
        ));
    }

    // ── Fix 3: Pre-rewrite block plumbing tests ────────────────

    #[test]
    fn test_archive_uses_request_blocks_not_engine() {
        // Block "b_archived" was previously archived → not in engine blocks.
        // But it IS in the current request (stateless client re-sent it).
        // Archive mutation should still find it and produce remove_turns.
        let engine_blocks: Vec<Block> = vec![mock_block("b_active", 1)]; // engine only has active block
        let request_blocks = vec![
            mock_block("b_active", 1),
            mock_block("b_archived", 3), // re-sent by client
        ];
        let mutations = vec![ContextMutation::Archive {
            block_id: "b_archived".into(),
        }];

        let decisions = apply_mutations(&engine_blocks, &request_blocks, &mutations);

        // Turn 3 should be marked for removal
        assert!(
            decisions.remove_turns.contains(&3),
            "Archive should use request_blocks for turn_index when block is not in engine"
        );
        assert_eq!(decisions.engine_updates.len(), 1);
        assert!(matches!(
            decisions.engine_updates[0].kind,
            EngineUpdateKind::Archive
        ));
    }

    #[test]
    fn test_expand_uses_engine_content_request_turn_index() {
        // Block "b1" exists in both engine and request, but at different turn_indices.
        // Expand should use engine's original content but request's turn_index.
        let mut engine_block = mock_block("b1", 4);
        engine_block.compressed_versions.original.content = "Full original content".to_string();
        let engine_blocks = vec![engine_block];

        let mut request_block = mock_block("b1", 7); // different turn_index in current request
        request_block.content = "Compressed content".to_string();
        let request_blocks = vec![request_block];

        let mutations = vec![ContextMutation::Expand {
            block_id: "b1".into(),
        }];

        let decisions = apply_mutations(&engine_blocks, &request_blocks, &mutations);

        // Should use request turn_index (7), not engine turn_index (4)
        assert_eq!(
            decisions.content_replacements.get(&7),
            Some(&"Full original content".to_string()),
            "Expand should use request block's turn_index for content replacement"
        );
        assert!(
            !decisions.content_replacements.contains_key(&4),
            "Should not use engine block's turn_index"
        );
    }

    #[test]
    fn test_turn_grouping_uses_request_blocks() {
        // Two blocks at turn 5 in request_blocks: "b1" and "b2".
        // Only "b1" is archived → turn should NOT be removed (partial archival).
        // "b2" only exists in request_blocks, not engine.
        let engine_blocks = vec![mock_block("b1", 5)];
        let request_blocks = vec![mock_block("b1", 5), mock_block("b2", 5)];
        let mutations = vec![ContextMutation::Archive {
            block_id: "b1".into(),
        }];

        let decisions = apply_mutations(&engine_blocks, &request_blocks, &mutations);

        // Turn 5 should NOT be removed because b2 (from request) is not archived
        assert!(
            !decisions.remove_turns.contains(&5),
            "Turn should not be removed when only some request blocks are archived"
        );
        // But a content stub should be generated
        assert_eq!(decisions.partial_turn_stubs.len(), 1);
        assert_eq!(decisions.partial_turn_stubs[0].turn_index, 5);
        assert_eq!(decisions.partial_turn_stubs[0].content_index, 0);
        // b1 should still have engine update
        assert_eq!(decisions.engine_updates.len(), 1);
    }

    #[test]
    fn test_turn_grouping_all_request_blocks_archived() {
        // Both blocks at turn 5 in request_blocks are archived → turn IS removed.
        let engine_blocks = vec![mock_block("b1", 5)]; // engine only knows b1
        let request_blocks = vec![mock_block("b1", 5), mock_block("b2", 5)];
        let mutations = vec![
            ContextMutation::Archive {
                block_id: "b1".into(),
            },
            ContextMutation::Archive {
                block_id: "b2".into(),
            },
        ];

        let decisions = apply_mutations(&engine_blocks, &request_blocks, &mutations);

        assert!(
            decisions.remove_turns.contains(&5),
            "Turn should be removed when ALL request blocks at that index are archived"
        );
        // No stubs needed when full turn is removed
        assert!(decisions.partial_turn_stubs.is_empty());
    }

    #[test]
    fn test_partial_turn_stubs_multiple_archived_blocks() {
        // 4 blocks at turn 3: archive b1 and b3, keep b2 and b4.
        let mut b1 = mock_block("b1", 3);
        b1.tokens = 5000;
        let mut b2 = mock_block("b2", 3);
        b2.tokens = 200;
        let mut b3 = mock_block("b3", 3);
        b3.tokens = 12000;
        let mut b4 = mock_block("b4", 3);
        b4.tokens = 100;

        let blocks = vec![b1, b2, b3, b4];
        let mutations = vec![
            ContextMutation::Archive {
                block_id: "b1".into(),
            },
            ContextMutation::Archive {
                block_id: "b3".into(),
            },
        ];

        let decisions = apply_mutations(&blocks, &blocks, &mutations);

        // Turn NOT removed (partial)
        assert!(!decisions.remove_turns.contains(&3));
        // Two stubs generated
        assert_eq!(decisions.partial_turn_stubs.len(), 2);
        // b1 at index 0
        assert_eq!(decisions.partial_turn_stubs[0].turn_index, 3);
        assert_eq!(decisions.partial_turn_stubs[0].content_index, 0);
        assert!(decisions.partial_turn_stubs[0].stub.contains("5.0k"));
        // b3 at index 2
        assert_eq!(decisions.partial_turn_stubs[1].turn_index, 3);
        assert_eq!(decisions.partial_turn_stubs[1].content_index, 2);
        assert!(decisions.partial_turn_stubs[1].stub.contains("12k"));
    }

    #[test]
    fn test_partial_turn_stubs_format_various_sizes() {
        assert_eq!(format_archive_stub(500), "[archived: 500 tokens]");
        assert_eq!(format_archive_stub(1500), "[archived: 1.5k tokens]");
        assert_eq!(format_archive_stub(12000), "[archived: 12k tokens]");
        assert_eq!(format_archive_stub(999), "[archived: 999 tokens]");
    }
}
