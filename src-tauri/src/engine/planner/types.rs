//! Core types for the context planner.
//!
//! Defines the input/output contracts for the planner, the mutation enum,
//! and the pending plan state used by `context_plan()`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;
use crate::engine::planner::file_tracker::FileMutation;
use crate::engine::types::BuiltInZone;

// ── Mutations ────────────────────────────────────────────────

/// A single context mutation the planner can apply between turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextMutation {
    /// Expand a compressed block back to full content.
    Expand { block_id: String },

    /// Remove a block from the active payload (kept in storage for recall).
    Archive { block_id: String },

    /// Bring an archived block back into active context.
    Recall { block_id: String },

    /// Pin a block to prevent auto-archival.
    Pin { block_id: String },

    /// Unpin a block (allow auto-archival).
    Unpin { block_id: String },

    /// Move a block to a different zone.
    Shift {
        block_id: String,
        target_zone: BuiltInZone,
    },

    /// Replace a block's content with a model-authored summary.
    Compress { block_id: String, summary: String },

    /// Split a thread at a given turn index, optionally archiving before the split.
    Split {
        thread_id: String,
        at_turn: u32,
        archive_before: bool,
    },

    /// Update a block's content (e.g. file mutation tracking).
    UpdateContent {
        block_id: String,
        new_content: String,
    },
}

// ── Heuristic Signals ────────────────────────────────────────

/// Signals collected from the environment for heuristic decisions.
#[derive(Debug, Clone, Default)]
pub struct HeuristicSignals {
    /// Current budget pressure (derived from budget status).
    pub budget_status: Option<BudgetStatus>,

    /// Files referenced in the current turn's tool calls.
    pub current_turn_files: Vec<String>,

    /// Files referenced in the previous turn's tool calls.
    pub previous_turn_files: Vec<String>,

    /// Current turn number (for staleness calculations).
    pub current_turn: u32,

    /// Whether a task boundary was detected (>50% new files).
    pub task_boundary_detected: bool,
}

// ── Planner Input ────────────────────────────────────────────

/// Snapshot of engine state provided to the planner for decision-making.
#[derive(Debug, Clone)]
pub struct PlannerInput {
    /// All active session blocks (ordered by zone + turn).
    pub blocks: Vec<Block>,

    /// Pending mutations from the model's `context_plan()` call (if any).
    pub pending_plan: Option<PendingPlan>,

    /// Heuristic signals from the environment.
    pub signals: HeuristicSignals,

    /// Budget status for the active session.
    pub budget: BudgetStatus,

    /// File mutations detected from tool calls (if any).
    pub file_mutations: Option<Vec<FileMutation>>,
}

// ── Pending Plan (from context_plan()) ───────────────────────

/// The model's planned context changes, validated and stored until turn end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPlan {
    /// Mutations to apply, in order.
    pub mutations: Vec<ContextMutation>,

    /// Preview of token impact (positive = savings, negative = growth).
    pub token_delta: i64,

    /// Preview of new block count after application.
    pub projected_block_count: usize,

    /// Preview of new budget utilization after application.
    pub projected_utilization: f64,
}

/// Raw actions from the `context_plan()` tool call, before validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanActions {
    #[serde(default)]
    pub expand: Vec<String>,

    #[serde(default)]
    pub archive: Vec<String>,

    #[serde(default)]
    pub recall: Vec<String>,

    #[serde(default)]
    pub pin: Vec<String>,

    #[serde(default)]
    pub unpin: Vec<String>,

    #[serde(default)]
    pub shift_to: HashMap<String, String>,

    #[serde(default)]
    pub compress: HashMap<String, String>,

    #[serde(default)]
    pub split: HashMap<String, SplitInstruction>,
}

/// Instructions for splitting a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitInstruction {
    pub at: u32,
    #[serde(default)]
    pub archive_before: bool,
}

// ── Planner Output ───────────────────────────────────────────

/// The planner's decisions for what to apply between turns.
#[derive(Debug, Clone)]
pub struct PlannerOutput {
    /// Mutations to apply to the engine state.
    pub mutations: Vec<ContextMutation>,

    /// Manifest text to inject into the next payload.
    pub manifest: Manifest,

    /// Cleanup instructions for ephemeral tool calls.
    pub cleanup: CleanupInstructions,
}

/// Manifest to inject into the conversation payload.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    /// Status line (~30 tok) — always present.
    pub status_line: String,

    /// Delta text (~50-100 tok) — only when context changed.
    pub delta: Option<String>,

    /// Warning text (~100-150 tok) — only at budget thresholds.
    pub warning: Option<String>,
}

impl Manifest {
    /// Combine all manifest sections into a single injection string.
    pub fn to_injection_text(&self) -> String {
        let mut parts = vec![self.status_line.clone()];
        if let Some(ref delta) = self.delta {
            parts.push(delta.clone());
        }
        if let Some(ref warning) = self.warning {
            parts.push(warning.clone());
        }
        parts.join("\n")
    }

    /// Check if there's any delta or warning content beyond the status line.
    pub fn has_changes(&self) -> bool {
        self.delta.is_some() || self.warning.is_some()
    }
}

/// Instructions for cleaning up ephemeral tool calls between turns.
#[derive(Debug, Clone, Default)]
pub struct CleanupInstructions {
    /// Whether any cleanup is needed.
    pub has_cleanup: bool,

    /// Breadcrumb text to insert in place of stripped tool calls.
    pub breadcrumb: Option<String>,

    /// Tool use IDs to strip from conversation history.
    pub tool_use_ids_to_strip: Vec<String>,
}

// ── Planner Configuration ────────────────────────────────────

/// Configuration for the context planner.
#[derive(Debug, Clone)]
pub struct PlannerConfig {
    /// User's budget ceiling as a fraction (0.0-1.0, default 0.80).
    pub budget_ceiling: f64,

    /// Soft threshold (fraction of ceiling). Start archiving stalest blocks.
    pub soft_threshold: f64,

    /// Medium threshold (fraction of ceiling). Archive middle-zone stale blocks.
    pub medium_threshold: f64,

    /// Hard threshold (fraction of ceiling). Emergency — protect only primacy + recency.
    pub hard_threshold: f64,

    /// Number of turns before a block is considered stale (default 10).
    pub staleness_turn_threshold: u32,

    /// Whether to inject manifests into payloads.
    pub manifest_enabled: bool,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            budget_ceiling: 0.80,
            soft_threshold: 0.50,
            medium_threshold: 0.80,
            hard_threshold: 1.00,
            staleness_turn_threshold: 10,
            manifest_enabled: true,
        }
    }
}

impl PlannerConfig {
    /// Absolute utilization threshold for soft level.
    pub fn soft_utilization(&self) -> f64 {
        self.budget_ceiling * self.soft_threshold
    }

    /// Absolute utilization threshold for medium level.
    pub fn medium_utilization(&self) -> f64 {
        self.budget_ceiling * self.medium_threshold
    }

    /// Absolute utilization threshold for hard level.
    pub fn hard_utilization(&self) -> f64 {
        self.budget_ceiling * self.hard_threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_mutation_variants_serialize() {
        let mutations = vec![
            ContextMutation::Expand {
                block_id: "b1".into(),
            },
            ContextMutation::Archive {
                block_id: "b2".into(),
            },
            ContextMutation::Recall {
                block_id: "b3".into(),
            },
            ContextMutation::Pin {
                block_id: "b4".into(),
            },
            ContextMutation::Unpin {
                block_id: "b5".into(),
            },
            ContextMutation::Shift {
                block_id: "b6".into(),
                target_zone: BuiltInZone::Primacy,
            },
            ContextMutation::Compress {
                block_id: "b7".into(),
                summary: "CSS styles".into(),
            },
            ContextMutation::Split {
                thread_id: "t1".into(),
                at_turn: 5,
                archive_before: true,
            },
        ];

        // All variants should round-trip through serde
        for mutation in &mutations {
            let json = serde_json::to_string(mutation).expect("serialize");
            let _back: ContextMutation = serde_json::from_str(&json).expect("deserialize");
        }
        assert_eq!(mutations.len(), 8);
    }

    #[test]
    fn test_plan_actions_deserialize_partial() {
        let json = r#"{"archive": ["b1", "b2"], "pin": ["b3"]}"#;
        let actions: PlanActions = serde_json::from_str(json).expect("deserialize");
        assert_eq!(actions.archive.len(), 2);
        assert_eq!(actions.pin.len(), 1);
        assert!(actions.expand.is_empty());
        assert!(actions.compress.is_empty());
    }

    #[test]
    fn test_plan_actions_deserialize_full() {
        let json = r#"{
            "expand": ["b1"],
            "archive": ["b2"],
            "recall": ["b3"],
            "pin": ["b4"],
            "unpin": ["b5"],
            "shift_to": {"b6": "primacy"},
            "compress": {"b7": "Summary text"},
            "split": {"t1": {"at": 5, "archive_before": true}}
        }"#;
        let actions: PlanActions = serde_json::from_str(json).expect("deserialize");
        assert_eq!(actions.expand, vec!["b1"]);
        assert_eq!(actions.archive, vec!["b2"]);
        assert_eq!(actions.recall, vec!["b3"]);
        assert_eq!(actions.shift_to.get("b6"), Some(&"primacy".to_string()));
        assert_eq!(
            actions.compress.get("b7"),
            Some(&"Summary text".to_string())
        );
        assert!(actions.split.get("t1").unwrap().archive_before);
    }

    #[test]
    fn test_manifest_to_injection_text_status_only() {
        let manifest = Manifest {
            status_line: "[Aperture: 45% budget | 12 blocks]".into(),
            delta: None,
            warning: None,
        };
        assert_eq!(
            manifest.to_injection_text(),
            "[Aperture: 45% budget | 12 blocks]"
        );
        assert!(!manifest.has_changes());
    }

    #[test]
    fn test_manifest_to_injection_text_with_delta_and_warning() {
        let manifest = Manifest {
            status_line: "[Aperture: 82% budget | 24 blocks]".into(),
            delta: Some("[Delta: archived #12, expanded #8]".into()),
            warning: Some("[Warning: approaching budget ceiling]".into()),
        };
        let text = manifest.to_injection_text();
        assert!(text.contains("82% budget"));
        assert!(text.contains("Delta:"));
        assert!(text.contains("Warning:"));
        assert!(manifest.has_changes());
    }

    #[test]
    fn test_planner_config_threshold_derivation() {
        let config = PlannerConfig::default();
        // ceiling=0.80, soft=0.50 of ceiling, medium=0.80 of ceiling, hard=1.0 of ceiling
        assert!((config.soft_utilization() - 0.40).abs() < f64::EPSILON);
        assert!((config.medium_utilization() - 0.64).abs() < f64::EPSILON);
        assert!((config.hard_utilization() - 0.80).abs() < f64::EPSILON);
    }

    #[test]
    fn test_pending_plan_round_trip() {
        let plan = PendingPlan {
            mutations: vec![
                ContextMutation::Archive {
                    block_id: "b1".into(),
                },
                ContextMutation::Compress {
                    block_id: "b2".into(),
                    summary: "Auth module summary".into(),
                },
            ],
            token_delta: 1500,
            projected_block_count: 10,
            projected_utilization: 0.52,
        };
        let json = serde_json::to_string(&plan).expect("serialize");
        let back: PendingPlan = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.mutations.len(), 2);
        assert_eq!(back.token_delta, 1500);
        assert!((back.projected_utilization - 0.52).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cleanup_instructions_default() {
        let cleanup = CleanupInstructions::default();
        assert!(!cleanup.has_cleanup);
        assert!(cleanup.breadcrumb.is_none());
        assert!(cleanup.tool_use_ids_to_strip.is_empty());
    }
}
