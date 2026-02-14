//! Context planner — the brain that ties context management together.
//!
//! The planner runs between turns, collecting signals from:
//! - Engine state (blocks, zones, staleness, tokens)
//! - Model intent (pending `context_plan()` actions)
//! - System heuristics (budget pressure, staleness, task detection)
//!
//! It produces mutations to apply, manifests to inject, and cleanup instructions.

pub mod applicator;
pub mod cleanup;
pub mod file_tracker;
pub mod heuristics;
pub mod manifest;
pub mod relevance;
pub mod types;

use std::collections::HashSet;
use std::sync::Mutex;

use tracing::debug;

use self::manifest::{build_manifest, TurnDelta};
use self::types::{
    CleanupInstructions, ContextMutation, HeuristicSignals, Manifest, PendingPlan, PlanActions,
    PlannerConfig, PlannerInput, PlannerOutput,
};
use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;
use crate::engine::types::BuiltInZone;

/// The context planner — runs between turns to manage context state.
pub struct ContextPlanner {
    config: PlannerConfig,
    /// The model's pending plan from the current turn (last-plan-wins).
    pending_plan: Mutex<Option<PendingPlan>>,
    /// Delta from the most recent plan execution (used for manifest generation).
    last_delta: Mutex<Option<TurnDelta>>,
    /// Runtime override for budget ceiling (set from UI settings).
    budget_ceiling_override: Mutex<Option<f64>>,
    /// Last turn's file set from real proxy traffic for task-boundary detection.
    previous_turn_files: Mutex<Vec<String>>,
}

impl ContextPlanner {
    pub fn new(config: PlannerConfig) -> Self {
        Self {
            config,
            pending_plan: Mutex::new(None),
            last_delta: Mutex::new(None),
            budget_ceiling_override: Mutex::new(None),
            previous_turn_files: Mutex::new(Vec::new()),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(PlannerConfig::default())
    }

    /// Get the current planner configuration.
    pub fn config(&self) -> &PlannerConfig {
        &self.config
    }

    /// Update planner configuration.
    pub fn set_config(&mut self, config: PlannerConfig) {
        self.config = config;
    }

    /// Get the effective budget ceiling (override or config default).
    pub fn effective_budget_ceiling(&self) -> f64 {
        let guard = self
            .budget_ceiling_override
            .lock()
            .expect("budget_ceiling lock");
        guard.unwrap_or(self.config.budget_ceiling)
    }

    /// Set a runtime override for the budget ceiling (from UI settings).
    pub fn set_budget_ceiling(&self, ceiling: f64) {
        let clamped = ceiling.clamp(0.40, 1.0);
        let mut guard = self
            .budget_ceiling_override
            .lock()
            .expect("budget_ceiling lock");
        *guard = Some(clamped);
    }

    // ── Plan Management ──────────────────────────────────────

    /// Store a pending plan from `context_plan()`. Last-plan-wins.
    pub fn set_pending_plan(&self, plan: PendingPlan) {
        let mut guard = self.pending_plan.lock().expect("pending_plan lock");
        *guard = Some(plan);
    }

    /// Take the pending plan (consuming it).
    pub fn take_pending_plan(&self) -> Option<PendingPlan> {
        let mut guard = self.pending_plan.lock().expect("pending_plan lock");
        guard.take()
    }

    /// Check if there's a pending plan.
    pub fn has_pending_plan(&self) -> bool {
        let guard = self.pending_plan.lock().expect("pending_plan lock");
        guard.is_some()
    }

    /// Get the last turn delta (for manifest generation on next turn).
    pub fn last_delta(&self) -> Option<TurnDelta> {
        let guard = self.last_delta.lock().expect("last_delta lock");
        guard.clone()
    }

    /// Build heuristic signals from real proxy traffic and planner state.
    pub fn build_heuristic_signals(
        &self,
        blocks: &[Block],
        budget: &BudgetStatus,
        current_turn_files: Vec<String>,
    ) -> HeuristicSignals {
        let mut unique_current = current_turn_files;
        unique_current.sort_unstable();
        unique_current.dedup();

        let previous = {
            let guard = self
                .previous_turn_files
                .lock()
                .expect("previous_turn_files lock");
            guard.clone()
        };
        let task_boundary = relevance::detect_task_boundary(&unique_current, &previous);
        {
            let mut guard = self
                .previous_turn_files
                .lock()
                .expect("previous_turn_files lock");
            *guard = unique_current.clone();
        }

        let current_turn = blocks
            .iter()
            .map(|b| b.metadata.turn_index)
            .max()
            .unwrap_or(0);

        HeuristicSignals {
            budget_status: Some(budget.clone()),
            current_turn_files: unique_current,
            previous_turn_files: previous,
            current_turn,
            task_boundary_detected: task_boundary,
        }
    }

    // ── Core Planning Logic ──────────────────────────────────

    /// Run the planner to produce output for between-turn application.
    ///
    /// This is the main entry point. Call with a snapshot of engine state,
    /// and get back mutations + manifest + cleanup instructions.
    pub fn plan(&self, input: &PlannerInput) -> PlannerOutput {
        let mut mutations = Vec::new();
        let mut effective_config = self.config.clone();
        effective_config.budget_ceiling = self.effective_budget_ceiling();

        // 1. Apply model's planned changes first (model intent takes priority)
        if let Some(ref plan) = input.pending_plan {
            debug!(
                "Applying model plan with {} mutations",
                plan.mutations.len()
            );
            mutations.extend(plan.mutations.clone());
        }

        // 2. Collect block IDs the model explicitly acted on (for conflict resolution)
        let model_acted_ids: HashSet<String> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Pin { block_id }
                | ContextMutation::Expand { block_id }
                | ContextMutation::Shift { block_id, .. }
                | ContextMutation::Compress { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .collect();

        // 3. Apply autonomous heuristics (budget pressure, staleness, relevance)
        let heuristic_mutations = heuristics::apply_heuristics(
            &input.blocks,
            &input.budget,
            &input.signals,
            &effective_config,
            &model_acted_ids,
        );
        mutations.extend(heuristic_mutations);

        // 4. Apply file mutation tracking
        if let Some(ref file_mutations) = input.file_mutations {
            let file_update_mutations =
                file_tracker::generate_file_update_mutations(file_mutations, &input.blocks);
            mutations.extend(file_update_mutations);
        }

        // 5. Build manifest (uses last turn's delta for the delta section)
        let last_delta = self.last_delta();
        let manifest = if self.config.manifest_enabled {
            build_manifest(&input.blocks, &input.budget, last_delta.as_ref())
        } else {
            Manifest::default()
        };

        // 6. Record this turn's delta for next time
        let net_delta = self.estimate_token_delta(&mutations, &input.blocks);
        let new_delta = if mutations.is_empty() {
            None
        } else {
            Some(TurnDelta::from_mutations(&mutations, net_delta))
        };
        {
            let mut guard = self.last_delta.lock().expect("last_delta lock");
            *guard = new_delta;
        }

        // 7. Build cleanup instructions with breadcrumb from applied mutations
        let budget_pct = input.budget.utilization;
        let breadcrumb = if mutations.is_empty() {
            None
        } else {
            let text = cleanup::generate_breadcrumb(&mutations, net_delta, budget_pct);
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        };
        let cleanup = CleanupInstructions {
            has_cleanup: !mutations.is_empty(),
            breadcrumb,
            tool_use_ids_to_strip: vec![], // Populated by runtime adapter during cleanup_history()
        };

        PlannerOutput {
            mutations,
            manifest,
            cleanup,
        }
    }

    /// Generate a manifest without running the full planner.
    /// Useful for `context_status()` tool responses.
    pub fn generate_manifest(&self, blocks: &[Block], budget: &BudgetStatus) -> Manifest {
        let last_delta = self.last_delta();
        build_manifest(blocks, budget, last_delta.as_ref())
    }

    /// Generate the full detailed manifest (for `context_status()` tool).
    pub fn generate_full_manifest(&self, blocks: &[Block], budget: &BudgetStatus) -> String {
        manifest::generate_full(blocks, budget)
    }

    // ── Validation ───────────────────────────────────────────

    /// Validate raw plan actions against current engine state.
    /// Returns a `PendingPlan` with validated mutations and projected impact.
    pub fn validate_plan(
        &self,
        actions: &PlanActions,
        blocks: &[Block],
        budget: &BudgetStatus,
    ) -> Result<PendingPlan, Vec<String>> {
        let mut mutations = Vec::new();
        let mut errors = Vec::new();

        let block_ids: std::collections::HashSet<&str> =
            blocks.iter().map(|b| b.id.as_str()).collect();

        // Validate expand actions
        for id in &actions.expand {
            if block_ids.contains(id.as_str()) {
                mutations.push(ContextMutation::Expand {
                    block_id: id.clone(),
                });
            } else {
                errors.push(format!("Block {id} not found for expand"));
            }
        }

        // Validate archive actions
        for id in &actions.archive {
            if block_ids.contains(id.as_str()) {
                mutations.push(ContextMutation::Archive {
                    block_id: id.clone(),
                });
            } else {
                errors.push(format!("Block {id} not found for archive"));
            }
        }

        // Validate recall actions (these reference archived blocks, not active ones)
        for id in &actions.recall {
            mutations.push(ContextMutation::Recall {
                block_id: id.clone(),
            });
        }

        // Validate pin actions
        for id in &actions.pin {
            if block_ids.contains(id.as_str()) {
                mutations.push(ContextMutation::Pin {
                    block_id: id.clone(),
                });
            } else {
                errors.push(format!("Block {id} not found for pin"));
            }
        }

        // Validate unpin actions
        for id in &actions.unpin {
            if block_ids.contains(id.as_str()) {
                mutations.push(ContextMutation::Unpin {
                    block_id: id.clone(),
                });
            } else {
                errors.push(format!("Block {id} not found for unpin"));
            }
        }

        // Validate shift_to actions
        for (id, zone_str) in &actions.shift_to {
            if !block_ids.contains(id.as_str()) {
                errors.push(format!("Block {id} not found for shift"));
                continue;
            }
            match parse_builtin_zone(zone_str) {
                Some(zone) => {
                    mutations.push(ContextMutation::Shift {
                        block_id: id.clone(),
                        target_zone: zone,
                    });
                }
                None => {
                    errors.push(format!("Invalid zone '{zone_str}' for shift"));
                }
            }
        }

        // Validate compress actions
        for (id, summary) in &actions.compress {
            if block_ids.contains(id.as_str()) {
                mutations.push(ContextMutation::Compress {
                    block_id: id.clone(),
                    summary: summary.clone(),
                });
            } else {
                errors.push(format!("Block {id} not found for compress"));
            }
        }

        // Validate split actions
        for (thread_id, instruction) in &actions.split {
            mutations.push(ContextMutation::Split {
                thread_id: thread_id.clone(),
                at_turn: instruction.at,
                archive_before: instruction.archive_before,
            });
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // Compute projected impact
        let token_delta = self.estimate_token_delta(&mutations, blocks);
        let archived_count = mutations
            .iter()
            .filter(|m| matches!(m, ContextMutation::Archive { .. }))
            .count();
        let recalled_count = mutations
            .iter()
            .filter(|m| matches!(m, ContextMutation::Recall { .. }))
            .count();

        let projected_block_count = blocks.len() - archived_count + recalled_count;
        let projected_tokens = (budget.used_tokens as i64 + token_delta).max(0) as u64;
        let projected_utilization = if budget.limit_tokens > 0 {
            projected_tokens as f64 / budget.limit_tokens as f64
        } else {
            0.0
        };

        Ok(PendingPlan {
            mutations,
            token_delta,
            projected_block_count,
            projected_utilization,
        })
    }

    // ── Internal Helpers ─────────────────────────────────────

    /// Estimate net token delta from a set of mutations against current blocks.
    fn estimate_token_delta(&self, mutations: &[ContextMutation], blocks: &[Block]) -> i64 {
        let mut delta: i64 = 0;

        for mutation in mutations {
            match mutation {
                ContextMutation::Archive { block_id } => {
                    if let Some(block) = blocks.iter().find(|b| b.id == *block_id) {
                        delta -= block.tokens as i64;
                    }
                }
                ContextMutation::Compress {
                    block_id, summary, ..
                } => {
                    if let Some(block) = blocks.iter().find(|b| b.id == *block_id) {
                        // Rough estimate: summary tokens ≈ summary.len() / 4
                        let summary_tokens = (summary.len() as i64) / 4;
                        delta -= block.tokens as i64;
                        delta += summary_tokens;
                    }
                }
                // Recall adds tokens (we don't know the exact amount without the archived block)
                ContextMutation::Recall { .. } => {
                    // Can't estimate without archived block data
                }
                // Expand, Pin, Unpin, Shift, Split, UpdateContent don't change total tokens
                _ => {}
            }
        }

        delta
    }
}

/// Parse a zone string into a BuiltInZone.
fn parse_builtin_zone(s: &str) -> Option<BuiltInZone> {
    match s.to_lowercase().as_str() {
        "primacy" => Some(BuiltInZone::Primacy),
        "middle" => Some(BuiltInZone::Middle),
        "recency" => Some(BuiltInZone::Recency),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::budget::AlertLevel;
    use crate::engine::types::{CompressionLevel, Role, Zone};

    fn mock_block(id: &str, role: Role, zone: BuiltInZone, tokens: u32) -> Block {
        Block {
            id: id.to_string(),
            role,
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

    #[test]
    fn test_planner_basic_plan_no_mutations() {
        let planner = ContextPlanner::with_default_config();
        let input = PlannerInput {
            blocks: vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)],
            pending_plan: None,
            signals: Default::default(),
            file_mutations: None,
            budget: mock_budget(50_000, 200_000),
        };

        let output = planner.plan(&input);
        assert!(output.mutations.is_empty());
        assert!(!output.manifest.status_line.is_empty());
    }

    #[test]
    fn test_planner_applies_model_plan() {
        let planner = ContextPlanner::with_default_config();
        let pending = PendingPlan {
            mutations: vec![
                ContextMutation::Archive {
                    block_id: "2".into(),
                },
                ContextMutation::Pin {
                    block_id: "1".into(),
                },
            ],
            token_delta: -500,
            projected_block_count: 1,
            projected_utilization: 0.45,
        };

        let input = PlannerInput {
            blocks: vec![
                mock_block("1", Role::User, BuiltInZone::Recency, 500),
                mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
            ],
            pending_plan: Some(pending),
            signals: Default::default(),
            file_mutations: None,
            budget: mock_budget(50_000, 200_000),
        };

        let output = planner.plan(&input);
        assert_eq!(output.mutations.len(), 2);
        assert!(matches!(
            &output.mutations[0],
            ContextMutation::Archive { block_id } if block_id == "2"
        ));
    }

    #[test]
    fn test_planner_last_plan_wins() {
        let planner = ContextPlanner::with_default_config();

        // Set first plan
        planner.set_pending_plan(PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "old".into(),
            }],
            token_delta: -100,
            projected_block_count: 1,
            projected_utilization: 0.5,
        });

        // Set second plan (replaces first)
        planner.set_pending_plan(PendingPlan {
            mutations: vec![ContextMutation::Pin {
                block_id: "new".into(),
            }],
            token_delta: 0,
            projected_block_count: 2,
            projected_utilization: 0.5,
        });

        let plan = planner.take_pending_plan().expect("should have plan");
        assert_eq!(plan.mutations.len(), 1);
        assert!(matches!(
            &plan.mutations[0],
            ContextMutation::Pin { block_id } if block_id == "new"
        ));
    }

    #[test]
    fn test_planner_records_delta_for_next_turn() {
        let planner = ContextPlanner::with_default_config();

        // First turn: no delta
        assert!(planner.last_delta().is_none());

        // Run with mutations
        let input = PlannerInput {
            blocks: vec![
                mock_block("1", Role::User, BuiltInZone::Recency, 500),
                mock_block("2", Role::Assistant, BuiltInZone::Middle, 1000),
            ],
            pending_plan: Some(PendingPlan {
                mutations: vec![ContextMutation::Archive {
                    block_id: "2".into(),
                }],
                token_delta: -1000,
                projected_block_count: 1,
                projected_utilization: 0.25,
            }),
            signals: Default::default(),
            file_mutations: None,
            budget: mock_budget(50_000, 200_000),
        };

        planner.plan(&input);

        // Now delta should be recorded
        let delta = planner.last_delta().expect("should have delta");
        assert_eq!(delta.archived_ids, vec!["2"]);
        assert!(delta.net_token_delta < 0);
    }

    #[test]
    fn test_runtime_budget_ceiling_override_affects_heuristics() {
        let planner = ContextPlanner::new(PlannerConfig {
            staleness_turn_threshold: 100,
            ..PlannerConfig::default()
        });

        let input = PlannerInput {
            blocks: vec![
                mock_block("1", Role::User, BuiltInZone::Recency, 500),
                mock_block("2", Role::Assistant, BuiltInZone::Middle, 1000),
            ],
            pending_plan: None,
            signals: Default::default(),
            file_mutations: None,
            budget: mock_budget(70_000, 200_000), // 35% utilization
        };

        // Default config: soft threshold is 40% (0.80 ceiling * 0.50 soft).
        let baseline = planner.plan(&input);
        assert!(
            baseline.mutations.is_empty(),
            "35% utilization should be below default soft threshold"
        );

        // Runtime ceiling override to 60% makes soft threshold 30%,
        // so 35% should now trigger soft-pressure archival.
        planner.set_budget_ceiling(0.60);
        let overridden = planner.plan(&input);
        assert!(
            overridden
                .mutations
                .iter()
                .any(|m| matches!(m, ContextMutation::Archive { .. })),
            "Runtime budget ceiling override should change heuristic behavior"
        );
    }

    #[test]
    fn test_validate_plan_valid_actions() {
        let planner = ContextPlanner::with_default_config();
        let blocks = vec![
            mock_block("1", Role::User, BuiltInZone::Recency, 500),
            mock_block("2", Role::Assistant, BuiltInZone::Middle, 1000),
        ];
        let budget = mock_budget(50_000, 200_000);

        let actions = PlanActions {
            archive: vec!["2".into()],
            pin: vec!["1".into()],
            ..Default::default()
        };

        let plan = planner.validate_plan(&actions, &blocks, &budget).unwrap();
        assert_eq!(plan.mutations.len(), 2);
        assert!(plan.token_delta < 0); // archiving saves tokens
        assert_eq!(plan.projected_block_count, 1);
    }

    #[test]
    fn test_validate_plan_invalid_block_id() {
        let planner = ContextPlanner::with_default_config();
        let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)];
        let budget = mock_budget(50_000, 200_000);

        let actions = PlanActions {
            archive: vec!["nonexistent".into()],
            ..Default::default()
        };

        let err = planner
            .validate_plan(&actions, &blocks, &budget)
            .unwrap_err();
        assert!(err[0].contains("nonexistent"));
    }

    #[test]
    fn test_validate_plan_invalid_zone() {
        let planner = ContextPlanner::with_default_config();
        let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)];
        let budget = mock_budget(50_000, 200_000);

        let actions = PlanActions {
            shift_to: [("1".into(), "invalid_zone".into())].into(),
            ..Default::default()
        };

        let err = planner
            .validate_plan(&actions, &blocks, &budget)
            .unwrap_err();
        assert!(err[0].contains("Invalid zone"));
    }

    #[test]
    fn test_estimate_token_delta_archive() {
        let planner = ContextPlanner::with_default_config();
        let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 1000)];
        let mutations = vec![ContextMutation::Archive {
            block_id: "1".into(),
        }];
        let delta = planner.estimate_token_delta(&mutations, &blocks);
        assert_eq!(delta, -1000);
    }

    #[test]
    fn test_estimate_token_delta_compress() {
        let planner = ContextPlanner::with_default_config();
        let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 1000)];
        let mutations = vec![ContextMutation::Compress {
            block_id: "1".into(),
            summary: "Short summary here.".into(), // ~20 chars -> ~5 tokens estimated
        }];
        let delta = planner.estimate_token_delta(&mutations, &blocks);
        // Should be negative (saving tokens)
        assert!(delta < 0);
    }

    #[test]
    fn test_manifest_disabled() {
        let config = PlannerConfig {
            manifest_enabled: false,
            ..Default::default()
        };
        let planner = ContextPlanner::new(config);
        let input = PlannerInput {
            blocks: vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)],
            pending_plan: None,
            signals: Default::default(),
            file_mutations: None,
            budget: mock_budget(50_000, 200_000),
        };

        let output = planner.plan(&input);
        assert!(output.manifest.status_line.is_empty());
    }

    #[test]
    fn test_parse_builtin_zone() {
        assert_eq!(parse_builtin_zone("primacy"), Some(BuiltInZone::Primacy));
        assert_eq!(parse_builtin_zone("MIDDLE"), Some(BuiltInZone::Middle));
        assert_eq!(parse_builtin_zone("Recency"), Some(BuiltInZone::Recency));
        assert_eq!(parse_builtin_zone("custom"), None);
    }

    #[test]
    fn test_planner_generates_breadcrumb_on_mutations() {
        let planner = ContextPlanner::with_default_config();
        let pending = PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "2".into(),
            }],
            token_delta: -500,
            projected_block_count: 1,
            projected_utilization: 0.25,
        };

        let input = PlannerInput {
            blocks: vec![
                mock_block("1", Role::User, BuiltInZone::Recency, 500),
                mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
            ],
            pending_plan: Some(pending),
            signals: Default::default(),
            file_mutations: None,
            budget: mock_budget(50_000, 200_000),
        };

        let output = planner.plan(&input);
        assert!(output.cleanup.has_cleanup);
        let breadcrumb = output
            .cleanup
            .breadcrumb
            .as_ref()
            .expect("should have breadcrumb");
        assert!(breadcrumb.contains("archived #2"));
        assert!(breadcrumb.contains("Budget: 25%"));
    }

    #[test]
    fn test_planner_no_breadcrumb_without_mutations() {
        let planner = ContextPlanner::with_default_config();
        let input = PlannerInput {
            blocks: vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)],
            pending_plan: None,
            signals: Default::default(),
            file_mutations: None,
            budget: mock_budget(50_000, 200_000),
        };

        let output = planner.plan(&input);
        assert!(!output.cleanup.has_cleanup);
        assert!(output.cleanup.breadcrumb.is_none());
    }

    // ── Heuristic Integration Tests ──────────────────────────

    #[test]
    fn test_planner_runs_heuristics_at_budget_pressure() {
        let planner = ContextPlanner::with_default_config();

        let mut blocks = Vec::new();
        for i in 0..5 {
            blocks.push(mock_block(
                &format!("m{i}"),
                Role::Assistant,
                BuiltInZone::Middle,
                1000,
            ));
            // Set turn_index to make blocks stale
            blocks.last_mut().unwrap().metadata.turn_index = i;
        }
        blocks.push(mock_block("recent", Role::User, BuiltInZone::Recency, 500));
        blocks.last_mut().unwrap().metadata.turn_index = 9;

        // 45% utilization — above soft threshold (40%)
        let input = PlannerInput {
            blocks,
            pending_plan: None,
            signals: types::HeuristicSignals {
                current_turn: 10,
                ..Default::default()
            },
            file_mutations: None,
            budget: mock_budget(90_000, 200_000),
        };

        let output = planner.plan(&input);

        // Heuristics should generate archival mutations
        let archive_count = output
            .mutations
            .iter()
            .filter(|m| matches!(m, ContextMutation::Archive { .. }))
            .count();
        assert!(
            archive_count > 0,
            "Expected heuristic archival at soft pressure"
        );
    }

    #[test]
    fn test_planner_model_pin_overrides_heuristic_archival() {
        let planner = ContextPlanner::with_default_config();

        let mut b1 = mock_block("b1", Role::Assistant, BuiltInZone::Middle, 2000);
        b1.metadata.turn_index = 0;
        let mut b2 = mock_block("b2", Role::Assistant, BuiltInZone::Middle, 1000);
        b2.metadata.turn_index = 1;

        // Model explicitly pins b1 — heuristics should not archive it
        let pending = PendingPlan {
            mutations: vec![ContextMutation::Pin {
                block_id: "b1".into(),
            }],
            token_delta: 0,
            projected_block_count: 2,
            projected_utilization: 0.45,
        };

        let input = PlannerInput {
            blocks: vec![b1, b2],
            pending_plan: Some(pending),
            signals: types::HeuristicSignals {
                current_turn: 10,
                ..Default::default()
            },
            file_mutations: None,
            budget: mock_budget(90_000, 200_000), // soft pressure
        };

        let output = planner.plan(&input);

        let archived_ids: Vec<&str> = output
            .mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        // b1 was pinned by model — should NOT be archived by heuristics
        assert!(
            !archived_ids.contains(&"b1"),
            "Model-pinned block should not be archived by heuristics"
        );
    }

    #[test]
    fn test_planner_file_mutations_generate_updates() {
        use crate::engine::planner::file_tracker::{FileMutation, FileMutationKind};

        let planner = ContextPlanner::with_default_config();

        let mut b1 = mock_block("b1", Role::ToolResult, BuiltInZone::Middle, 500);
        b1.metadata.file_paths = vec!["src/auth.rs".to_string()];
        b1.metadata.turn_index = 3;

        let input = PlannerInput {
            blocks: vec![b1],
            pending_plan: None,
            signals: Default::default(),
            file_mutations: Some(vec![FileMutation {
                file_path: "src/auth.rs".to_string(),
                kind: FileMutationKind::Edit,
                new_content: Some("fn updated_auth() {}".to_string()),
            }]),
            budget: mock_budget(50_000, 200_000),
        };

        let output = planner.plan(&input);

        let update_mutations: Vec<_> = output
            .mutations
            .iter()
            .filter(|m| matches!(m, ContextMutation::UpdateContent { .. }))
            .collect();

        assert_eq!(update_mutations.len(), 1);
        assert!(matches!(
            &update_mutations[0],
            ContextMutation::UpdateContent { block_id, new_content }
                if block_id == "b1" && new_content == "fn updated_auth() {}"
        ));
    }

    #[test]
    fn test_planner_no_heuristics_below_threshold() {
        let planner = ContextPlanner::with_default_config();

        let mut b1 = mock_block("b1", Role::Assistant, BuiltInZone::Middle, 1000);
        b1.metadata.turn_index = 0;

        let input = PlannerInput {
            blocks: vec![b1],
            pending_plan: None,
            signals: types::HeuristicSignals {
                current_turn: 5,
                ..Default::default()
            },
            file_mutations: None,
            budget: mock_budget(20_000, 200_000), // 10% — well below thresholds
        };

        let output = planner.plan(&input);

        // No budget pressure, staleness threshold not reached (5 < 10)
        assert!(output.mutations.is_empty());
    }

    #[test]
    fn test_build_heuristic_signals_tracks_previous_turn_files_and_boundaries() {
        let planner = ContextPlanner::with_default_config();
        let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 100)];
        let budget = mock_budget(10_000, 200_000);

        let first =
            planner.build_heuristic_signals(&blocks, &budget, vec!["src/auth.rs".to_string()]);
        assert!(first.task_boundary_detected);
        assert!(first.previous_turn_files.is_empty());
        assert_eq!(first.current_turn_files, vec!["src/auth.rs".to_string()]);

        let second = planner.build_heuristic_signals(
            &blocks,
            &budget,
            vec!["src/new.rs".to_string(), "src/other.rs".to_string()],
        );
        assert!(second.task_boundary_detected);
        assert_eq!(second.previous_turn_files, vec!["src/auth.rs".to_string()]);
    }

    #[test]
    fn test_build_heuristic_signals_normalizes_current_turn_files() {
        let planner = ContextPlanner::with_default_config();
        let blocks = vec![mock_block("1", Role::Assistant, BuiltInZone::Middle, 100)];
        let budget = mock_budget(10_000, 200_000);

        let signals = planner.build_heuristic_signals(
            &blocks,
            &budget,
            vec![
                "src/b.rs".to_string(),
                "src/a.rs".to_string(),
                "src/b.rs".to_string(),
            ],
        );

        assert_eq!(
            signals.current_turn_files,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
        assert_eq!(signals.current_turn, blocks[0].metadata.turn_index);
    }
}
