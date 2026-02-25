//! Context engine — manages universal blocks, zones, sessions, and policies.
//!
//! The `ContextEngine` is the central coordinator for all context management.
//! It receives parsed blocks from the proxy, processes them (zone assignment,
//! token counting, dependency tracking), and provides a policy-checked mutation API.

pub mod action_log;
pub mod block;
pub mod budget;
pub mod compression;
pub mod dependency;
mod ingest;
pub mod pipeline;
pub mod planner;
pub mod policy;
pub mod session;
mod session_sync;
pub mod staleness;
pub mod storage;
pub mod store;
pub mod tokens;
pub mod types;
pub mod versioning;
pub mod zone;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tracing::{info, warn};

use self::action_log::{ActionActor, ActionKind, ActionLog, ActionRecord};
use self::block::Block;
use self::budget::{BudgetConfig, BudgetStatus};
use self::compression::CompressionSettings;
use self::dependency::{build_dependencies, DependencyEdge, DependencyGraph};
use self::pipeline::{classify, ClassificationResult, PipelineConfig};
use self::planner::ContextPlanner;
use self::policy::{PolicyDecision, PolicyEngine, ProposedAction};
use self::session::{SessionInfo, SessionStore};
use self::storage::SqliteStorage;
use self::store::BlockStore;
use self::tokens::count_tokens;
use self::types::{CompressionLevel, PinPosition, Role, Zone};
use self::versioning::{BlockVersion, EditSource, VersionStore};
use crate::events::dispatcher::DynDispatcher;

/// Central context engine coordinating all subsystems.
pub struct ContextEngine {
    pub store: BlockStore,
    pub sessions: SessionStore,
    pub versions: VersionStore,
    pub dependencies: DependencyGraph,
    pub action_log: ActionLog,
    pub policy: PolicyEngine,
    pub planner: ContextPlanner,
    compression_settings: Mutex<CompressionSettings>,
    session_identity_index: DashMap<String, String>,
    pipeline_config: PipelineConfig,
    dispatcher: Option<DynDispatcher>,
    db: Option<SqliteStorage>,
}

/// Result of block ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResult {
    pub session_id: String,
    pub block_count: usize,
    pub total_tokens: u32,
    pub alert_level: budget::AlertLevel,
    /// Whether the ingest actually replaced session blocks. False when the
    /// regressive-subset guard skips the ingest to avoid transient collapses.
    pub applied: bool,
}

impl ContextEngine {
    /// Create a new engine with optional event dispatcher and database.
    pub fn new(dispatcher: Option<DynDispatcher>) -> Self {
        let db = match SqliteStorage::open(&storage::default_db_path()) {
            Ok(db) => {
                info!(
                    "SQLite storage initialized at {:?}",
                    storage::default_db_path()
                );
                Some(db)
            }
            Err(e) => {
                warn!("Failed to open SQLite storage: {e}. Running in-memory only.");
                None
            }
        };

        Self {
            store: BlockStore::new(),
            sessions: SessionStore::new(),
            versions: VersionStore::new(),
            dependencies: DependencyGraph::new(),
            action_log: ActionLog::new(),
            policy: PolicyEngine::new(),
            planner: ContextPlanner::with_default_config(),
            compression_settings: Mutex::new(CompressionSettings::default()),
            session_identity_index: DashMap::new(),
            pipeline_config: PipelineConfig::default(),
            dispatcher,
            db,
        }
    }

    /// Create an engine without persistence (for testing).
    pub fn new_in_memory(dispatcher: Option<DynDispatcher>) -> Self {
        Self {
            store: BlockStore::new(),
            sessions: SessionStore::new(),
            versions: VersionStore::new(),
            dependencies: DependencyGraph::new(),
            action_log: ActionLog::new(),
            policy: PolicyEngine::new(),
            planner: ContextPlanner::with_default_config(),
            compression_settings: Mutex::new(CompressionSettings::default()),
            session_identity_index: DashMap::new(),
            pipeline_config: PipelineConfig::default(),
            dispatcher,
            db: None,
        }
    }

    // ── Queries ──────────────────────────────────────────────

    /// All blocks in the current active session, ordered by zone + turn.
    pub fn active_session_blocks(&self) -> Vec<Block> {
        if let Some(session) = self.sessions.active() {
            let mut blocks = self.store.get_many(&session.block_ids);
            zone::sort_by_context_order(&mut blocks);
            blocks
        } else {
            self.store.all_ordered()
        }
    }

    /// All blocks in a specific session, ordered by zone + turn.
    pub fn session_blocks(&self, session_id: &str) -> Vec<Block> {
        if let Some(session) = self.sessions.get(session_id) {
            let mut blocks = self.store.get_many(&session.block_ids);
            zone::sort_by_context_order(&mut blocks);
            blocks
        } else {
            Vec::new()
        }
    }

    /// All blocks in the store (unfiltered).
    pub fn all_blocks(&self) -> Vec<Block> {
        self.store.all_ordered()
    }

    /// Get a single block by ID.
    pub fn block(&self, id: &str) -> Option<Block> {
        self.store.get(id)
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions.list()
    }

    /// Get the active session info.
    pub fn active_session(&self) -> Option<SessionInfo> {
        self.sessions.active().map(|s| s.info())
    }

    /// Active session ID.
    pub fn active_session_id(&self) -> Option<String> {
        self.sessions.active_id()
    }

    /// Whether a session exists.
    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.get(session_id).is_some()
    }

    /// Budget status for the active session.
    ///
    /// Includes tool definition overhead in the used count so budget utilization
    /// reflects what the LLM actually counts toward its context window.
    pub fn budget_status(&self) -> BudgetStatus {
        let session = self.sessions.active();
        let (used, limit) = match &session {
            Some(s) => (
                s.total_tokens.saturating_add(s.overhead_tokens),
                s.token_budget,
            ),
            None => (self.store.total_tokens(), 200_000),
        };
        budget::budget_status(used, limit, &BudgetConfig::default())
    }

    /// Budget status for a specific session.
    pub fn session_budget_status(&self, session_id: &str) -> BudgetStatus {
        let (used, limit) = match self.sessions.get(session_id) {
            Some(s) => (
                s.total_tokens.saturating_add(s.overhead_tokens),
                s.token_budget,
            ),
            None => (0, 200_000),
        };
        budget::budget_status(used, limit, &BudgetConfig::default())
    }

    /// Current sidekick compression settings.
    pub fn compression_settings(&self) -> CompressionSettings {
        match self.compression_settings.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                warn!("compression_settings lock poisoned, returning default");
                CompressionSettings::default()
            }
        }
    }

    /// Update sidekick compression settings.
    pub fn set_compression_settings(&self, settings: CompressionSettings) {
        let normalized = settings.normalized();
        match self.compression_settings.lock() {
            Ok(mut guard) => *guard = normalized,
            Err(_) => warn!("compression_settings lock poisoned, cannot update settings"),
        }
    }

    /// Version history for a block.
    pub fn block_versions(&self, block_id: &str) -> Vec<BlockVersion> {
        self.versions.history(block_id)
    }

    /// Dependencies of a block.
    pub fn block_dependencies(&self, block_id: &str) -> Vec<DependencyEdge> {
        self.dependencies.dependencies_of(block_id)
    }

    /// Dependents of a block (what depends on it).
    pub fn block_dependents(&self, block_id: &str) -> Vec<DependencyEdge> {
        self.dependencies.dependents_of(block_id)
    }

    /// Recent action log entries.
    pub fn recent_actions(&self, n: usize) -> Vec<ActionRecord> {
        self.action_log.recent(n)
    }

    /// Run classification pipeline and return results.
    pub fn classify(&self) -> ClassificationResult {
        let mut blocks = self.active_session_blocks();
        classify(&mut blocks, &self.pipeline_config)
    }

    // ── Mutations (Policy-Checked) ───────────────────────────

    /// Update block content. Goes through policy + versioning.
    pub fn update_content(
        &self,
        block_id: &str,
        new_content: &str,
        model: &str,
        confirmed: bool,
    ) -> Result<PolicyDecision, String> {
        let block = self.store.get(block_id).ok_or("Block not found")?;
        if block.content == new_content {
            return Ok(PolicyDecision::Allow);
        }

        let action = ProposedAction::EditContent {
            block_id: block_id.to_string(),
        };
        let decision = self.policy.check(&action, Some(&block));

        if matches!(decision, PolicyDecision::Deny { .. }) {
            return Ok(decision);
        }
        if matches!(decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
            return Ok(decision);
        }

        // Record old version
        self.versions.record(
            block_id,
            block.content.clone(),
            block.tokens,
            EditSource::User,
        );

        // Apply edit. If caller doesn't know the model yet, fall back to active session model.
        let token_model = if model.is_empty() || model == "unknown" {
            self.sessions
                .active()
                .map(|s| s.model)
                .unwrap_or_else(|| model.to_string())
        } else {
            model.to_string()
        };
        let new_tokens = count_tokens(new_content, &token_model);
        self.store.update(block_id, |b| {
            b.content = new_content.to_string();
            b.tokens = new_tokens;
        });
        self.refresh_active_session_totals();

        // Log action
        let mut record = action_log::new_record(
            ActionActor::User,
            ActionKind::EditContent,
            vec![block_id.to_string()],
            format!(
                "Edited block content ({} → {} tokens)",
                block.tokens, new_tokens
            ),
        );
        record.undoable = true;
        record.undo_data = Some(serde_json::to_string(&block.content).unwrap_or_default());
        self.action_log.record(record);

        // Emit update
        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();

        Ok(PolicyDecision::Allow)
    }

    /// Move a block to a different zone.
    pub fn move_block(
        &self,
        block_id: &str,
        target_zone: Zone,
        confirmed: bool,
    ) -> Result<PolicyDecision, String> {
        let block = self.store.get(block_id).ok_or("Block not found")?;
        if block.zone == target_zone {
            return Ok(PolicyDecision::Allow);
        }

        let action = ProposedAction::MoveBlock {
            block_id: block_id.to_string(),
            target_zone: format!("{target_zone:?}"),
        };
        let decision = self.policy.check(&action, Some(&block));

        if matches!(decision, PolicyDecision::Deny { .. }) {
            return Ok(decision);
        }
        if matches!(decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
            return Ok(decision);
        }

        let old_zone = block.zone.clone();
        self.store.update(block_id, |b| {
            b.zone = target_zone.clone();
        });

        let mut record = action_log::new_record(
            ActionActor::User,
            ActionKind::MoveZone,
            vec![block_id.to_string()],
            format!("Moved block from {old_zone:?} to {target_zone:?}"),
        );
        record.undoable = true;
        record.undo_data = Some(serde_json::to_string(&old_zone).unwrap_or_default());
        self.action_log.record(record);

        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();
        Ok(PolicyDecision::Allow)
    }

    /// Pin or unpin a block.
    pub fn pin_block(
        &self,
        block_id: &str,
        position: Option<PinPosition>,
    ) -> Result<PolicyDecision, String> {
        let block = self.store.get(block_id).ok_or("Block not found")?;
        if block.pinned == position {
            return Ok(PolicyDecision::Allow);
        }

        let old_pin = block.pinned;
        self.store.update(block_id, |b| {
            b.pinned = position;
        });

        let mut record = action_log::new_record(
            ActionActor::User,
            ActionKind::Pin,
            vec![block_id.to_string()],
            format!("Pin changed: {old_pin:?} → {position:?}"),
        );
        record.undoable = true;
        record.undo_data = Some(serde_json::to_string(&old_pin).unwrap_or_default());
        self.action_log.record(record);

        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();
        Ok(PolicyDecision::Allow)
    }

    /// Compress a block to a specified level.
    pub fn compress_block(
        &self,
        block_id: &str,
        level: CompressionLevel,
        confirmed: bool,
    ) -> Result<PolicyDecision, String> {
        let block = self.store.get(block_id).ok_or("Block not found")?;
        if block.compression_level == level {
            return Ok(PolicyDecision::Allow);
        }

        let action = ProposedAction::CompressBlock {
            block_id: block_id.to_string(),
            level,
        };
        let decision = self.policy.check(&action, Some(&block));

        if matches!(decision, PolicyDecision::Deny { .. }) {
            return Ok(decision);
        }
        if matches!(decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
            return Ok(decision);
        }

        // Record version before compression
        self.versions.record(
            block_id,
            block.content.clone(),
            block.tokens,
            EditSource::Compression,
        );

        self.store.update(block_id, |b| {
            b.compression_level = level;
        });

        self.action_log.record(action_log::new_record(
            ActionActor::User,
            ActionKind::Compress,
            vec![block_id.to_string()],
            format!("Compressed to {level:?}"),
        ));

        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();
        Ok(PolicyDecision::Allow)
    }

    /// Remove a block.
    pub fn remove_block(&self, block_id: &str, confirmed: bool) -> Result<PolicyDecision, String> {
        let block = self.store.get(block_id).ok_or("Block not found")?;

        let action = ProposedAction::RemoveBlock {
            block_id: block_id.to_string(),
        };
        let decision = self.policy.check(&action, Some(&block));

        if matches!(decision, PolicyDecision::Deny { .. }) {
            return Ok(decision);
        }
        if matches!(decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
            return Ok(decision);
        }

        self.store.remove(block_id);
        self.dependencies.remove_block(block_id);
        self.versions.remove(block_id);
        self.refresh_active_session_totals();

        let mut record = action_log::new_record(
            ActionActor::User,
            ActionKind::Remove,
            vec![block_id.to_string()],
            format!("Removed block (was {} tokens)", block.tokens),
        );
        record.undoable = true;
        record.undo_data = Some(serde_json::to_string(&block).unwrap_or_default());
        self.action_log.record(record);

        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();
        Ok(PolicyDecision::Allow)
    }

    /// Move multiple blocks to a zone atomically.
    pub fn bulk_move(
        &self,
        block_ids: &[String],
        target_zone: Zone,
        confirmed: bool,
    ) -> Result<PolicyDecision, String> {
        if block_ids.is_empty() {
            return Ok(PolicyDecision::Allow);
        }

        let mut blocks: Vec<Block> = Vec::with_capacity(block_ids.len());
        for id in block_ids {
            let block = self
                .store
                .get(id)
                .ok_or_else(|| format!("Block not found: {id}"))?;
            blocks.push(block);
        }

        let moving_ids: Vec<String> = blocks
            .iter()
            .filter(|block| block.zone != target_zone)
            .map(|block| block.id.clone())
            .collect();
        if moving_ids.is_empty() {
            return Ok(PolicyDecision::Allow);
        }

        for block in &blocks {
            if block.zone == target_zone {
                continue;
            }
            let decision = self.policy.check(
                &ProposedAction::MoveBlock {
                    block_id: block.id.clone(),
                    target_zone: format!("{target_zone:?}"),
                },
                Some(block),
            );
            if matches!(decision, PolicyDecision::Deny { .. }) {
                return Ok(decision);
            }
            if matches!(decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
                return Ok(decision);
            }
        }

        for id in &moving_ids {
            self.store.update(id, |b| {
                b.zone = target_zone.clone();
            });
        }

        let mut record = action_log::new_record(
            ActionActor::User,
            ActionKind::MoveZone,
            moving_ids.clone(),
            format!("Bulk moved {} blocks to {target_zone:?}", moving_ids.len()),
        );
        record.undoable = true;
        self.action_log.record(record);

        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();
        Ok(PolicyDecision::Allow)
    }

    /// Remove multiple blocks atomically.
    pub fn bulk_remove(
        &self,
        block_ids: &[String],
        confirmed: bool,
    ) -> Result<PolicyDecision, String> {
        if block_ids.is_empty() {
            return Ok(PolicyDecision::Allow);
        }

        let bulk_decision = self.policy.check(
            &ProposedAction::BulkRemove {
                block_ids: block_ids.to_vec(),
            },
            None,
        );
        if matches!(bulk_decision, PolicyDecision::Deny { .. }) {
            return Ok(bulk_decision);
        }
        if matches!(bulk_decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
            return Ok(bulk_decision);
        }

        let mut blocks: Vec<Block> = Vec::with_capacity(block_ids.len());
        for id in block_ids {
            let block = self
                .store
                .get(id)
                .ok_or_else(|| format!("Block not found: {id}"))?;
            blocks.push(block);
        }

        for block in &blocks {
            let decision = self.policy.check(
                &ProposedAction::RemoveBlock {
                    block_id: block.id.clone(),
                },
                Some(block),
            );
            if matches!(decision, PolicyDecision::Deny { .. }) {
                return Ok(decision);
            }
            if matches!(decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
                return Ok(decision);
            }
        }

        for id in block_ids {
            self.store.remove(id);
            self.dependencies.remove_block(id);
            self.versions.remove(id);
        }
        self.refresh_active_session_totals();

        let mut record = action_log::new_record(
            ActionActor::User,
            ActionKind::BulkRemove,
            block_ids.to_vec(),
            format!("Removed {} blocks", block_ids.len()),
        );
        record.undoable = true;
        self.action_log.record(record);

        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();
        Ok(PolicyDecision::Allow)
    }

    // ── Session Management ───────────────────────────────────

    /// Switch to a different session.
    pub fn switch_session(&self, session_id: &str) -> bool {
        self.sessions.switch_to(session_id)
    }

    /// Create a new session manually.
    pub fn create_session(
        &self,
        provider: &str,
        model: &str,
        source: &str,
        thread_id: Option<&str>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let token_budget = budget::default_token_limit(model);
        self.sessions.create(
            id.clone(),
            provider.to_string(),
            model.to_string(),
            source.to_string(),
            thread_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
            token_budget,
        );

        if let Some(ref db) = self.db {
            if let Some(session) = self.sessions.get(&id) {
                if let Err(e) = db.save_session(&session) {
                    warn!("Failed to persist session: {e}");
                }
            }
        }

        id
    }

    /// Clear all in-memory context state across all sessions.
    pub fn clear_all_sessions(&self, confirmed: bool) -> Result<PolicyDecision, String> {
        let session_id = self
            .sessions
            .active_id()
            .unwrap_or_else(|| "__all__".to_string());
        let action = ProposedAction::ClearSession { session_id };
        let decision = self.policy.check(&action, None);

        if matches!(decision, PolicyDecision::Deny { .. }) {
            return Ok(decision);
        }
        if matches!(decision, PolicyDecision::RequireConfirmation { .. }) && !confirmed {
            return Ok(decision);
        }

        let session_ids: Vec<String> = self.sessions.list().into_iter().map(|s| s.id).collect();
        let removed_block_ids = self.store.ids();

        if let Some(ref db) = self.db {
            for session_id in &session_ids {
                if let Err(e) = db.delete_session(session_id) {
                    warn!("Failed to delete session {session_id} during clear: {e}");
                }
            }
        }

        self.store.clear();
        self.sessions.clear();
        self.versions.clear();
        self.dependencies.clear();
        self.session_identity_index.clear();
        self.action_log.clear();
        self.planner.clear_all_session_state();

        self.action_log.record(action_log::new_record(
            ActionActor::User,
            ActionKind::ClearSession,
            removed_block_ids,
            format!("Cleared {} sessions", session_ids.len()),
        ));

        self.emit_context_updated(0, 0);
        Ok(PolicyDecision::Allow)
    }

    // ── Undo ─────────────────────────────────────────────────

    /// Undo the last undoable action for a block.
    pub fn undo_block(&self, block_id: &str) -> Result<(), String> {
        let (content, tokens) = self
            .versions
            .undo_content(block_id)
            .ok_or("No previous version to undo to")?;

        self.store.update(block_id, |b| {
            b.content = content;
            b.tokens = tokens;
        });
        self.refresh_active_session_totals();

        let (bc, tt) = self.active_session_counts();
        self.emit_context_updated(bc, tt);
        self.persist_active_session();
        Ok(())
    }

    // ── System-Driven Block Mutations ─────────────────────────
    // These bypass policy checks — used by the planner/rewriter for
    // engine-side updates (zone shifts, pin toggles) that are system-driven.

    /// Move a block to a different zone (system-driven, no policy check).
    pub fn move_block_internal(&self, block_id: &str, target_zone: Zone) {
        let Some(block) = self.store.get(block_id) else {
            return;
        };
        if block.zone == target_zone {
            return;
        }
        self.store.update(block_id, |b| {
            b.zone = target_zone.clone();
        });
    }

    /// Set pin state on a block (system-driven, no policy check).
    pub fn set_pin_internal(&self, block_id: &str, position: Option<PinPosition>) {
        let Some(block) = self.store.get(block_id) else {
            return;
        };
        if block.pinned == position {
            return;
        }
        self.store.update(block_id, |b| {
            b.pinned = position;
        });
    }

    /// Archive a block (system-driven, no policy check).
    pub fn archive_block_internal(&self, block_id: &str) {
        if !self.store.contains(block_id) {
            return;
        }
        self.store.remove(block_id);
        self.dependencies.remove_block(block_id);
        self.versions.remove(block_id);
        self.refresh_active_session_totals();
    }

    /// Apply a model-authored compression summary to a block.
    pub fn apply_compression_summary_internal(&self, block_id: &str, summary: &str) {
        let Some(block) = self.store.get(block_id) else {
            return;
        };
        if block.content == summary && block.compression_level == CompressionLevel::Summarized {
            return;
        }

        let token_model = self
            .sessions
            .active()
            .map(|s| s.model)
            .unwrap_or_else(|| "unknown".to_string());
        let summary_tokens = count_tokens(summary, &token_model);
        let summary_content = summary.to_string();

        self.store.update(block_id, |b| {
            b.content = summary_content.clone();
            b.tokens = summary_tokens;
            b.compression_level = CompressionLevel::Summarized;
            b.compressed_versions.summarized = Some(block::CompressionVersion {
                content: summary_content.clone(),
                tokens: summary_tokens,
            });
        });
        self.refresh_active_session_totals();
    }

    /// Restore a compressed block back to its original content.
    pub fn restore_original_internal(&self, block_id: &str) {
        let Some(block) = self.store.get(block_id) else {
            return;
        };
        if block.compression_level == CompressionLevel::Original
            && block.content == block.compressed_versions.original.content
        {
            return;
        }

        let original_content = block.compressed_versions.original.content;
        let original_tokens = block.compressed_versions.original.tokens;
        self.store.update(block_id, |b| {
            b.content = original_content.clone();
            b.tokens = original_tokens;
            b.compression_level = CompressionLevel::Original;
        });
        self.refresh_active_session_totals();
    }

    /// Update block content to reflect file mutations from real tool traffic.
    pub fn update_content_internal(&self, block_id: &str, new_content: &str) {
        let Some(block) = self.store.get(block_id) else {
            return;
        };
        if block.content == new_content && block.compressed_versions.original.content == new_content
        {
            return;
        }

        let token_model = self
            .sessions
            .active()
            .map(|s| s.model)
            .unwrap_or_else(|| "unknown".to_string());
        let new_tokens = count_tokens(new_content, &token_model);
        let new_content_owned = new_content.to_string();

        self.store.update(block_id, |b| {
            b.content = new_content_owned.clone();
            b.tokens = new_tokens;
            b.compression_level = CompressionLevel::Original;
            b.compressed_versions.original = block::CompressionVersion {
                content: new_content_owned.clone(),
                tokens: new_tokens,
            };
            b.compressed_versions.trimmed = None;
            b.compressed_versions.summarized = None;
            b.compressed_versions.minimal = None;
        });
        self.refresh_active_session_totals();
    }

    // ── Internal Helpers ─────────────────────────────────────

    /// Ensure a session exists for this provider/model combo,
    /// creating one if needed. Returns the session ID.
    fn ensure_session(
        &self,
        provider: &str,
        model: &str,
        source: &str,
        thread_id: Option<&str>,
    ) -> String {
        let identity_key = session_identity_key(provider, model, source, thread_id);
        if let Some(existing_id) = self
            .session_identity_index
            .get(&identity_key)
            .map(|entry| entry.value().clone())
        {
            if self.sessions.get(&existing_id).is_some() {
                // Only flip active session if the existing session matches the
                // current active's model, or the active session is small.
                // This prevents auxiliary model sessions (e.g. Haiku classifier)
                // from stealing active status from the main conversation.
                let should_activate = match self.sessions.active() {
                    None => true,
                    Some(active) => {
                        active.total_tokens < 1000
                            || (active.provider == provider && active.model == model)
                    }
                };
                if should_activate {
                    self.sessions.switch_to(&existing_id);
                }
                return existing_id;
            }
            self.session_identity_index.remove(&identity_key);
        }

        let session_id = self.create_session(provider, model, source, thread_id);
        self.session_identity_index
            .insert(identity_key, session_id.clone());
        session_id
    }

    /// Resolve the session ID for a provider/model/source/thread identity,
    /// creating the session if needed.
    pub fn resolve_session(
        &self,
        provider: &str,
        model: &str,
        source: &str,
        thread_id: Option<&str>,
    ) -> String {
        self.ensure_session(provider, model, source, thread_id)
    }
}

fn session_identity_key(
    provider: &str,
    model: &str,
    source: &str,
    thread_id: Option<&str>,
) -> String {
    let normalized_thread = thread_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");
    format!("{provider}|{model}|{source}|{normalized_thread}")
}

impl Default for ContextEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests;
