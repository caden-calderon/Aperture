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
mod validation;

use std::collections::HashSet;
use std::sync::Mutex;

use dashmap::DashMap;
use tracing::{debug, warn};

use self::manifest::{build_manifest, TurnDelta};
use self::types::{
    ArchivalSuggestion, CleanupInstructions, ContextMutation, HeuristicSignals, Manifest,
    PendingPlan, PlannerConfig, PlannerInput, PlannerOutput,
};
use crate::engine::block::Block;
use crate::engine::budget::{AlertLevel, BudgetStatus};
use crate::engine::types::BuiltInZone;

#[derive(Debug, Clone)]
struct PlannerSessionState {
    pending_plan: Option<PendingPlan>,
    staged_plan: Option<PendingPlan>,
    persistent_archived_ids: HashSet<String>,
    last_delta: Option<TurnDelta>,
    previous_turn_files: Vec<String>,
    last_alert_level: AlertLevel,
}

impl Default for PlannerSessionState {
    fn default() -> Self {
        Self {
            pending_plan: None,
            staged_plan: None,
            persistent_archived_ids: HashSet::new(),
            last_delta: None,
            previous_turn_files: Vec::new(),
            last_alert_level: AlertLevel::Normal,
        }
    }
}

const LEGACY_SESSION_ID: &str = "__legacy__";

/// The context planner — runs between turns to manage context state.
pub struct ContextPlanner {
    config: PlannerConfig,
    /// Per-session mutable planner state.
    session_states: DashMap<String, PlannerSessionState>,
    /// Runtime override for budget ceiling (set from UI settings).
    budget_ceiling_override: Mutex<Option<f64>>,
}

impl ContextPlanner {
    pub fn new(config: PlannerConfig) -> Self {
        Self {
            config,
            session_states: DashMap::new(),
            budget_ceiling_override: Mutex::new(None),
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

    fn with_session_state<R>(
        &self,
        session_id: &str,
        mut f: impl FnMut(&mut PlannerSessionState) -> R,
    ) -> R {
        let mut entry = self
            .session_states
            .entry(session_id.to_string())
            .or_default();
        f(entry.value_mut())
    }

    fn read_session_state<R>(
        &self,
        session_id: &str,
        f: impl FnOnce(&PlannerSessionState) -> R,
    ) -> R {
        if let Some(entry) = self.session_states.get(session_id) {
            f(entry.value())
        } else {
            let default_state = PlannerSessionState::default();
            f(&default_state)
        }
    }

    /// Clear planner mutable state for a single session.
    pub fn clear_session_state(&self, session_id: &str) {
        self.session_states.remove(session_id);
    }

    /// Clear planner mutable state for all sessions.
    pub fn clear_all_session_state(&self) {
        self.session_states.clear();
    }

    // ── Plan Management ──────────────────────────────────────

    /// Store a pending plan from `context_plan()`. Last-plan-wins.
    pub fn set_pending_plan_for_session(&self, session_id: &str, plan: PendingPlan) {
        self.with_session_state(session_id, |state| {
            state.pending_plan = Some(plan.clone());
        });
    }

    /// Take the pending plan (consuming it).
    pub fn take_pending_plan_for_session(&self, session_id: &str) -> Option<PendingPlan> {
        self.with_session_state(session_id, |state| state.pending_plan.take())
    }

    /// Check if there's a pending plan.
    pub fn has_pending_plan_for_session(&self, session_id: &str) -> bool {
        self.read_session_state(session_id, |state| state.pending_plan.is_some())
    }

    /// Find the effective session that has a staged plan.
    ///
    /// Strict isolation rule: staged plans never fall back to other sessions.
    /// If `preferred_session_id` has no staged plan, return `None`.
    fn find_staged_plan_session(&self, preferred_session_id: &str) -> Option<String> {
        self.session_states.get(preferred_session_id).and_then(|s| {
            s.staged_plan
                .as_ref()
                .map(|_| preferred_session_id.to_string())
        })
    }

    /// Replace the staged plan with a validated plan.
    pub fn set_staged_plan_for_session(&self, session_id: &str, plan: PendingPlan) {
        self.with_session_state(session_id, |state| {
            state.staged_plan = Some(plan.clone());
        });
    }

    /// Append additional validated mutations to the staged plan (last-write-wins per mutation slot).
    pub fn append_staged_plan_for_session(
        &self,
        session_id: &str,
        plan: PendingPlan,
        blocks: &[Block],
        budget: &BudgetStatus,
    ) -> PendingPlan {
        let merged_mutations = if let Some(existing) = self.staged_plan_for_session(session_id) {
            Self::merge_mutations(&existing.mutations, &plan.mutations)
        } else {
            plan.mutations
        };
        let merged = self.project_pending_plan_from_mutations(merged_mutations, blocks, budget);
        self.set_staged_plan_for_session(session_id, merged.clone());
        merged
    }

    /// Get a snapshot of the staged plan for `session_id`.
    pub fn staged_plan_for_session(&self, session_id: &str) -> Option<PendingPlan> {
        let effective = self.find_staged_plan_session(session_id)?;
        self.read_session_state(&effective, |state| state.staged_plan.clone())
    }

    /// Check if a staged plan exists for `session_id`.
    pub fn has_staged_plan_for_session(&self, session_id: &str) -> bool {
        self.staged_plan_for_session(session_id)
            .as_ref()
            .map(|plan| !plan.mutations.is_empty())
            .unwrap_or(false)
    }

    /// Clear staged plan for `session_id`. Returns true if a staged plan existed.
    pub fn clear_staged_plan_for_session(&self, session_id: &str) -> bool {
        match self.find_staged_plan_session(session_id) {
            Some(effective) => {
                self.with_session_state(&effective, |state| state.staged_plan.take().is_some())
            }
            None => false,
        }
    }

    /// Promote staged plan to pending plan so it is applied on the next planner run.
    pub fn commit_staged_plan_for_session(&self, session_id: &str) -> Option<PendingPlan> {
        let effective = self.find_staged_plan_session(session_id)?;
        self.with_session_state(&effective, |state| {
            let staged = state.staged_plan.take();
            if let Some(plan) = staged.clone() {
                state.pending_plan = Some(plan);
            }
            staged
        })
    }

    /// Eagerly persist archive IDs from committed mutations so they survive
    /// even if the pending plan is never consumed by `plan_for_session()`.
    ///
    /// This closes the gap where `commit_staged_plan_for_session()` stores
    /// the pending plan but archive IDs only persist when `plan_for_session()`
    /// runs. If the pending plan is not consumed (session mismatch or streaming
    /// race), the archive intent would be lost without this call.
    ///
    /// Safe: `persistent_archived_ids` is a `HashSet`, so duplicate inserts
    /// are no-ops, and `already_archived` in `plan_for_session()` prevents
    /// double-application in the rewriter.
    pub fn add_persistent_archives_for_session(
        &self,
        session_id: &str,
        mutations: &[ContextMutation],
    ) {
        self.with_session_state(session_id, |state| {
            for mutation in mutations {
                match mutation {
                    ContextMutation::Archive { block_id } => {
                        state.persistent_archived_ids.insert(block_id.clone());
                    }
                    ContextMutation::Recall { block_id } => {
                        state.persistent_archived_ids.remove(block_id);
                    }
                    _ => {}
                }
            }
        });
    }

    /// Get the last turn delta (for manifest generation on next turn).
    pub fn last_delta_for_session(&self, session_id: &str) -> Option<TurnDelta> {
        self.read_session_state(session_id, |state| state.last_delta.clone())
    }

    // Legacy wrappers retained for unit tests and non-session-aware callers.
    pub fn set_pending_plan(&self, plan: PendingPlan) {
        self.set_pending_plan_for_session(LEGACY_SESSION_ID, plan);
    }

    pub fn take_pending_plan(&self) -> Option<PendingPlan> {
        self.take_pending_plan_for_session(LEGACY_SESSION_ID)
    }

    pub fn has_pending_plan(&self) -> bool {
        self.has_pending_plan_for_session(LEGACY_SESSION_ID)
    }

    pub fn set_staged_plan(&self, plan: PendingPlan) {
        self.set_staged_plan_for_session(LEGACY_SESSION_ID, plan);
    }

    pub fn append_staged_plan(
        &self,
        plan: PendingPlan,
        blocks: &[Block],
        budget: &BudgetStatus,
    ) -> PendingPlan {
        self.append_staged_plan_for_session(LEGACY_SESSION_ID, plan, blocks, budget)
    }

    pub fn staged_plan(&self) -> Option<PendingPlan> {
        self.staged_plan_for_session(LEGACY_SESSION_ID)
    }

    pub fn has_staged_plan(&self) -> bool {
        self.has_staged_plan_for_session(LEGACY_SESSION_ID)
    }

    pub fn clear_staged_plan(&self) -> bool {
        self.clear_staged_plan_for_session(LEGACY_SESSION_ID)
    }

    pub fn commit_staged_plan(&self) -> Option<PendingPlan> {
        self.commit_staged_plan_for_session(LEGACY_SESSION_ID)
    }

    pub fn last_delta(&self) -> Option<TurnDelta> {
        self.last_delta_for_session(LEGACY_SESSION_ID)
    }

    /// Build heuristic signals from real proxy traffic and planner state.
    pub fn build_heuristic_signals_for_session(
        &self,
        session_id: &str,
        blocks: &[Block],
        budget: &BudgetStatus,
        current_turn_files: Vec<String>,
    ) -> HeuristicSignals {
        let mut unique_current = current_turn_files;
        unique_current.sort_unstable();
        unique_current.dedup();

        let previous =
            self.read_session_state(session_id, |state| state.previous_turn_files.clone());
        let task_boundary = relevance::detect_task_boundary(&unique_current, &previous);
        self.with_session_state(session_id, |state| {
            state.previous_turn_files = unique_current.clone();
        });

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

    pub fn build_heuristic_signals(
        &self,
        blocks: &[Block],
        budget: &BudgetStatus,
        current_turn_files: Vec<String>,
    ) -> HeuristicSignals {
        self.build_heuristic_signals_for_session(
            LEGACY_SESSION_ID,
            blocks,
            budget,
            current_turn_files,
        )
    }

    /// Generate archival suggestions (non-executing) based on heuristics.
    /// Returns candidate block IDs with metadata for LLM review.
    pub fn generate_archival_suggestions(
        &self,
        blocks: &[Block],
        budget: &BudgetStatus,
        signals: &HeuristicSignals,
    ) -> Vec<ArchivalSuggestion> {
        let mut effective_config = self.config.clone();
        effective_config.budget_ceiling = self.effective_budget_ceiling();

        heuristics::collect_archival_candidates(blocks, budget, signals, &effective_config)
    }

    /// Build preview-time heuristic signals from persisted per-session traffic state.
    pub fn preview_signals_for_session(
        &self,
        session_id: &str,
        blocks: &[Block],
        budget: &BudgetStatus,
    ) -> HeuristicSignals {
        let last_turn_files =
            self.read_session_state(session_id, |state| state.previous_turn_files.clone());
        let current_turn = blocks
            .iter()
            .map(|b| b.metadata.turn_index)
            .max()
            .unwrap_or(0);

        HeuristicSignals {
            budget_status: Some(budget.clone()),
            current_turn_files: last_turn_files.clone(),
            previous_turn_files: last_turn_files,
            current_turn,
            task_boundary_detected: false,
        }
    }

    // ── Pressure Level Tracking ────────────────────────────────
    //
    // Pressure levels are derived from the planner's OWN thresholds
    // (which respect the budget ceiling slider), NOT the hardcoded
    // AlertLevel in BudgetStatus.

    /// Compute the current pressure level from planner config thresholds.
    ///
    /// Uses the effective budget ceiling (including UI override), so when
    /// the user drags the ceiling slider to 50%, the thresholds shift:
    ///   soft = 25%, medium = 40%, hard = 50%
    fn pressure_level(&self, utilization: f64) -> AlertLevel {
        let mut effective = self.config.clone();
        effective.budget_ceiling = self.effective_budget_ceiling();

        if utilization >= effective.hard_utilization() {
            AlertLevel::Emergency
        } else if utilization >= effective.medium_utilization() {
            AlertLevel::Critical
        } else if utilization >= effective.soft_utilization() {
            AlertLevel::Warning
        } else {
            AlertLevel::Normal
        }
    }

    /// Check if pressure level has changed since last check.
    /// Returns a warning message if crossing a threshold, None otherwise.
    /// Updates the stored level on change.
    pub fn check_alert_level_change_for_session(
        &self,
        session_id: &str,
        budget: &BudgetStatus,
        blocks: &[Block],
        signals: &HeuristicSignals,
    ) -> Option<String> {
        let current = self.pressure_level(budget.utilization);

        let previous = self.read_session_state(session_id, |state| state.last_alert_level);

        if current == previous {
            return None;
        }

        self.with_session_state(session_id, |state| {
            state.last_alert_level = current;
        });

        // Only warn on escalation (not on recovery — recovery is good news, no action needed)
        if current <= previous {
            return None;
        }

        // Generate suggestions to include in warning
        let suggestions = self.generate_archival_suggestions(blocks, budget, signals);
        let stale_middle: Vec<_> = suggestions.iter().filter(|s| s.tier.is_primary()).collect();
        let suggestion_count = stale_middle.len();
        let suggestion_tokens: u32 = stale_middle.iter().map(|s| s.tokens).sum();

        let pct = (budget.utilization * 100.0) as u32;
        let remaining = budget.remaining_tokens;
        let ceiling_pct = (self.effective_budget_ceiling() * 100.0) as u32;

        let message = match current {
            AlertLevel::Warning => format!(
                "[Aperture: context at {pct}% (soft threshold of {ceiling_pct}% ceiling) — {remaining} tokens remaining. {} stale middle-zone blocks (~{} tokens) suggested for archival. Consider cleaning after current task.]",
                suggestion_count, format_tokens(suggestion_tokens)
            ),
            AlertLevel::Critical => format!(
                "[Aperture: context at {pct}% (medium threshold of {ceiling_pct}% ceiling) — {remaining} tokens remaining. {} stale middle-zone blocks (~{} tokens) suggested for archival. Pause and reorganize context now.]",
                suggestion_count, format_tokens(suggestion_tokens)
            ),
            AlertLevel::Emergency => format!(
                "[Aperture: EMERGENCY — context at {pct}% (hard threshold = {ceiling_pct}% ceiling) — {remaining} tokens remaining. {} stale middle-zone blocks MUST be archived to prevent overflow. Call aperture_context_plan immediately.]",
                suggestion_count
            ),
            AlertLevel::Normal => return None, // Can't escalate TO normal
        };

        debug!(
            "Pressure level escalated: {:?} → {:?} ({}%, ceiling {}%)",
            previous, current, pct, ceiling_pct
        );
        Some(message)
    }

    pub fn check_alert_level_change(
        &self,
        budget: &BudgetStatus,
        blocks: &[Block],
        signals: &HeuristicSignals,
    ) -> Option<String> {
        self.check_alert_level_change_for_session(LEGACY_SESSION_ID, budget, blocks, signals)
    }

    /// Determine if this turn is a batch point where heuristic mutations should apply.
    ///
    /// Batch points:
    /// - Pressure level just changed (threshold crossing based on ceiling slider)
    /// - Explicit pending plan from LLM (plan commit)
    /// - Task boundary detected (file set changed completely)
    pub fn is_batch_point_for_session(
        &self,
        session_id: &str,
        budget: &BudgetStatus,
        signals: &HeuristicSignals,
        has_pending_plan: bool,
    ) -> bool {
        // Explicit model plan commit is always a batch point
        if has_pending_plan {
            return true;
        }

        // Task boundary detection (file set changed completely)
        if signals.task_boundary_detected {
            return true;
        }

        // Pressure level change (checked without updating — just peek)
        let current = self.pressure_level(budget.utilization);
        self.read_session_state(session_id, |state| current != state.last_alert_level)
    }

    pub fn is_batch_point(
        &self,
        budget: &BudgetStatus,
        signals: &HeuristicSignals,
        has_pending_plan: bool,
    ) -> bool {
        self.is_batch_point_for_session(LEGACY_SESSION_ID, budget, signals, has_pending_plan)
    }

    // ── Core Planning Logic ──────────────────────────────────

    /// Run the planner to produce output for between-turn application.
    ///
    /// This is the main entry point. Call with a snapshot of engine state,
    /// and get back mutations + manifest + cleanup instructions.
    pub fn plan_for_session(&self, session_id: &str, input: &PlannerInput) -> PlannerOutput {
        let mut mutations = Vec::new();
        let mut persistent_archived =
            self.read_session_state(session_id, |state| state.persistent_archived_ids.clone());

        // 1. Apply model's planned changes first (model intent takes priority)
        if let Some(ref plan) = input.pending_plan {
            warn!(
                session_id = %session_id,
                mutations = plan.mutations.len(),
                persistent_count = persistent_archived.len(),
                "R9-DIAG planner: applying pending plan for session"
            );
            mutations.extend(plan.mutations.clone());

            // Model-authored archive/recall should persist beyond a single rewrite pass.
            for mutation in &plan.mutations {
                match mutation {
                    ContextMutation::Archive { block_id } => {
                        persistent_archived.insert(block_id.clone());
                    }
                    ContextMutation::Recall { block_id } => {
                        persistent_archived.remove(block_id);
                    }
                    _ => {}
                }
            }
            self.with_session_state(session_id, |state| {
                state.persistent_archived_ids = persistent_archived.clone();
            });
        }

        // Re-apply persistent archive intent for any blocks that reappear in
        // stateless/full-history clients (Claude/Codex tool loops).
        //
        // Use request_block_ids (parsed from the ORIGINAL request body before rewriting)
        // instead of engine blocks. Archived blocks are removed from the engine by
        // archive_block_internal(), so they won't appear in input.blocks. But stateless
        // clients re-send them every request, so they DO appear in request_block_ids.
        if !persistent_archived.is_empty() {
            let active_ids: &HashSet<String> = &input.request_block_ids;
            let mut already_archived: HashSet<String> = mutations
                .iter()
                .filter_map(|m| match m {
                    ContextMutation::Archive { block_id } => Some(block_id.clone()),
                    _ => None,
                })
                .collect();

            for block_id in &persistent_archived {
                if active_ids.contains(block_id.as_str()) && !already_archived.contains(block_id) {
                    mutations.push(ContextMutation::Archive {
                        block_id: block_id.clone(),
                    });
                    already_archived.insert(block_id.clone());
                }
            }
        }

        // 2. Autonomous heuristics are now DISABLED.
        //
        // Heuristics generate archival SUGGESTIONS (not mutations).
        // The LLM controls all context mutations via aperture_context_plan.
        //
        // Suggestions are surfaced to the LLM via:
        //   - Threshold warnings (check_alert_level_change)
        //   - aperture_context_preview tool (suggested_archival field)
        //
        // This prevents the "death spiral" where autonomous archival triggers
        // cache invalidation → budget spike → more archival → more cache misses.
        debug!("Autonomous heuristics disabled — LLM controls context via staged planning");

        // 3. Apply file mutation tracking
        if let Some(ref file_mutations) = input.file_mutations {
            let file_update_mutations =
                file_tracker::generate_file_update_mutations(file_mutations, &input.blocks);
            mutations.extend(file_update_mutations);
        }

        // 4. Build manifest (uses last turn's delta for the delta section)
        let last_delta = self.last_delta_for_session(session_id);
        let manifest = if self.config.manifest_enabled {
            build_manifest(&input.blocks, &input.budget, last_delta.as_ref())
        } else {
            Manifest::default()
        };

        // 5. Record this turn's delta for next time
        let net_delta = self.estimate_token_delta(&mutations, &input.blocks);
        let new_delta = if mutations.is_empty() {
            None
        } else {
            Some(TurnDelta::from_mutations(&mutations, net_delta))
        };
        self.with_session_state(session_id, |state| {
            state.last_delta = new_delta.clone();
        });

        // 6. Build cleanup instructions with breadcrumb from applied mutations
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

    pub fn plan(&self, input: &PlannerInput) -> PlannerOutput {
        self.plan_for_session(LEGACY_SESSION_ID, input)
    }

    /// Generate a manifest without running the full planner.
    /// Useful for `context_status()` tool responses.
    pub fn generate_manifest_for_session(
        &self,
        session_id: &str,
        blocks: &[Block],
        budget: &BudgetStatus,
    ) -> Manifest {
        let last_delta = self.last_delta_for_session(session_id);
        build_manifest(blocks, budget, last_delta.as_ref())
    }

    pub fn generate_manifest(&self, blocks: &[Block], budget: &BudgetStatus) -> Manifest {
        let last_delta = self.last_delta_for_session(LEGACY_SESSION_ID);
        build_manifest(blocks, budget, last_delta.as_ref())
    }

    /// Generate the full detailed manifest (for `context_status()` tool).
    pub fn generate_full_manifest(&self, blocks: &[Block], budget: &BudgetStatus) -> String {
        manifest::generate_full(blocks, budget)
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

/// Format token count for display.
fn format_tokens(tokens: u32) -> String {
    if tokens >= 1000 {
        let k = tokens as f64 / 1000.0;
        if k >= 10.0 {
            format!("{}k", k.round() as u32)
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{tokens}")
    }
}

#[cfg(test)]
mod tests;
