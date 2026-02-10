//! Mutation policy engine.
//!
//! Centralized guardrails for destructive actions (compress, archive, remove).
//! Every mutation goes through policy checks before execution.

use serde::{Deserialize, Serialize};

use super::block::Block;
use super::types::{CompressionLevel, PinPosition, Role};

/// Policy decision for a proposed action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Action is allowed to proceed.
    Allow,
    /// Action is blocked with a reason.
    Deny { reason: String },
    /// Action needs user confirmation.
    RequireConfirmation { reason: String },
}

/// An action that can be checked against policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProposedAction {
    /// Remove a block entirely.
    RemoveBlock { block_id: String },
    /// Compress a block to a specific level.
    CompressBlock {
        block_id: String,
        level: CompressionLevel,
    },
    /// Move a block to a different zone.
    MoveBlock {
        block_id: String,
        target_zone: String,
    },
    /// Edit block content.
    EditContent { block_id: String },
    /// Remove multiple blocks at once.
    BulkRemove { block_ids: Vec<String> },
    /// Clear all blocks in a session.
    ClearSession { session_id: String },
}

/// Policy engine with configurable rules.
pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
}

/// A single policy rule that can allow, deny, or require confirmation.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub name: String,
    pub check: fn(&ProposedAction, Option<&Block>) -> Option<PolicyDecision>,
}

impl PolicyEngine {
    /// Create with default safety rules.
    pub fn new() -> Self {
        Self {
            rules: default_rules(),
        }
    }

    /// Check an action against all rules.
    /// Returns the strictest decision (Deny > RequireConfirmation > Allow).
    pub fn check(&self, action: &ProposedAction, block: Option<&Block>) -> PolicyDecision {
        let mut strictest = PolicyDecision::Allow;

        for rule in &self.rules {
            if let Some(decision) = (rule.check)(action, block) {
                match (&strictest, &decision) {
                    (PolicyDecision::Allow, _) => strictest = decision,
                    (PolicyDecision::RequireConfirmation { .. }, PolicyDecision::Deny { .. }) => {
                        strictest = decision;
                    }
                    _ => {} // keep current strictest
                }
            }
        }

        strictest
    }

    /// Add a custom policy rule.
    pub fn add_rule(&mut self, rule: PolicyRule) {
        self.rules.push(rule);
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Default Rules ────────────────────────────────────────────

fn default_rules() -> Vec<PolicyRule> {
    vec![
        PolicyRule {
            name: "protect_pinned".into(),
            check: protect_pinned,
        },
        PolicyRule {
            name: "protect_system".into(),
            check: protect_system,
        },
        PolicyRule {
            name: "warn_bulk_remove".into(),
            check: warn_bulk_remove,
        },
        PolicyRule {
            name: "warn_session_clear".into(),
            check: warn_session_clear,
        },
    ]
}

/// Pinned blocks require confirmation before removal/compression.
fn protect_pinned(action: &ProposedAction, block: Option<&Block>) -> Option<PolicyDecision> {
    let block = block?;
    block.pinned?;

    match action {
        ProposedAction::RemoveBlock { .. } => Some(PolicyDecision::RequireConfirmation {
            reason: format!(
                "Block is pinned to {:?}. Removing a pinned block may lose important context.",
                block.pinned.unwrap_or(PinPosition::Top)
            ),
        }),
        ProposedAction::CompressBlock { .. } => Some(PolicyDecision::RequireConfirmation {
            reason: "Block is pinned. Compressing may alter important context.".into(),
        }),
        _ => None,
    }
}

/// System prompts require confirmation before removal.
fn protect_system(action: &ProposedAction, block: Option<&Block>) -> Option<PolicyDecision> {
    let block = block?;
    if block.role != Role::System {
        return None;
    }

    match action {
        ProposedAction::RemoveBlock { .. } => Some(PolicyDecision::RequireConfirmation {
            reason: "Removing system prompt blocks may change model behavior.".into(),
        }),
        _ => None,
    }
}

/// Bulk removal of many blocks requires confirmation.
fn warn_bulk_remove(action: &ProposedAction, _block: Option<&Block>) -> Option<PolicyDecision> {
    if let ProposedAction::BulkRemove { block_ids } = action {
        if block_ids.len() > 5 {
            return Some(PolicyDecision::RequireConfirmation {
                reason: format!(
                    "About to remove {} blocks at once. This cannot be undone.",
                    block_ids.len()
                ),
            });
        }
    }
    None
}

/// Session clear requires confirmation.
fn warn_session_clear(action: &ProposedAction, _block: Option<&Block>) -> Option<PolicyDecision> {
    if let ProposedAction::ClearSession { .. } = action {
        return Some(PolicyDecision::RequireConfirmation {
            reason: "Clearing a session removes all blocks. This cannot be undone.".into(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, Zone};

    fn make_block(role: Role, pinned: Option<PinPosition>) -> Block {
        Block {
            id: "b1".to_string(),
            role,
            block_type: None,
            content: "test".to_string(),
            tokens: 100,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "test".to_string(),
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
                turn_index: 0,
                tool_name: None,
                file_paths: vec![],
            },
        }
    }

    #[test]
    fn test_normal_block_removal_allowed() {
        let engine = PolicyEngine::new();
        let block = make_block(Role::User, None);
        let action = ProposedAction::RemoveBlock {
            block_id: "b1".into(),
        };

        assert_eq!(engine.check(&action, Some(&block)), PolicyDecision::Allow);
    }

    #[test]
    fn test_pinned_block_removal_needs_confirmation() {
        let engine = PolicyEngine::new();
        let block = make_block(Role::User, Some(PinPosition::Top));
        let action = ProposedAction::RemoveBlock {
            block_id: "b1".into(),
        };

        assert!(matches!(
            engine.check(&action, Some(&block)),
            PolicyDecision::RequireConfirmation { .. }
        ));
    }

    #[test]
    fn test_system_block_removal_needs_confirmation() {
        let engine = PolicyEngine::new();
        let block = make_block(Role::System, None);
        let action = ProposedAction::RemoveBlock {
            block_id: "b1".into(),
        };

        assert!(matches!(
            engine.check(&action, Some(&block)),
            PolicyDecision::RequireConfirmation { .. }
        ));
    }

    #[test]
    fn test_bulk_remove_small_set_allowed() {
        let engine = PolicyEngine::new();
        let action = ProposedAction::BulkRemove {
            block_ids: vec!["b1".into(), "b2".into()],
        };

        assert_eq!(engine.check(&action, None), PolicyDecision::Allow);
    }

    #[test]
    fn test_bulk_remove_large_set_needs_confirmation() {
        let engine = PolicyEngine::new();
        let action = ProposedAction::BulkRemove {
            block_ids: (0..10).map(|i| format!("b{i}")).collect(),
        };

        assert!(matches!(
            engine.check(&action, None),
            PolicyDecision::RequireConfirmation { .. }
        ));
    }

    #[test]
    fn test_session_clear_needs_confirmation() {
        let engine = PolicyEngine::new();
        let action = ProposedAction::ClearSession {
            session_id: "s1".into(),
        };

        assert!(matches!(
            engine.check(&action, None),
            PolicyDecision::RequireConfirmation { .. }
        ));
    }
}
