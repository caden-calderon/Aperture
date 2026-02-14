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
pub mod pipeline;
pub mod planner;
pub mod policy;
pub mod session;
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
use tracing::{debug, info, warn};

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
use self::types::{CompressionLevel, PinPosition, Zone};
use self::versioning::{BlockVersion, EditSource, VersionStore};
use crate::events::dispatcher::DynDispatcher;
use crate::events::types::ApertureEvent;

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

    // ── Block Ingestion ──────────────────────────────────────

    /// Ingest blocks from a proxy capture.
    ///
    /// This is the main entry point called by the proxy handler after
    /// parsing a request/response exchange.
    pub fn ingest(
        &self,
        provider: &str,
        model: &str,
        source: &str,
        thread_id: Option<&str>,
        mut request_blocks: Vec<Block>,
        mut response_blocks: Vec<Block>,
    ) -> IngestResult {
        // Ensure/get session
        let session_id = self.ensure_session(provider, model, source, thread_id);

        // Response parsers currently emit turn_index=0 for assistant/tool output.
        // Normalize response turns so latest response content is classified and
        // ordered after the latest request turn in this exchange.
        if !request_blocks.is_empty() && !response_blocks.is_empty() {
            let next_turn = request_blocks
                .iter()
                .map(|b| b.metadata.turn_index)
                .max()
                .unwrap_or(0)
                .saturating_add(1);

            for block in &mut response_blocks {
                block.metadata.turn_index = next_turn;
                block.last_referenced_turn = next_turn;
            }
        }

        // Recount tokens accurately
        for block in request_blocks.iter_mut().chain(response_blocks.iter_mut()) {
            block.tokens = count_tokens(&block.content, model);
        }

        // Combine all blocks for this exchange
        let mut all_blocks: Vec<Block> = Vec::new();
        all_blocks.append(&mut request_blocks);
        all_blocks.append(&mut response_blocks);

        // Remove old session blocks — each API request contains the full
        // conversation history with new UUIDs, so we replace rather than accumulate.
        let old_block_ids = self
            .sessions
            .get(&session_id)
            .map(|s| s.block_ids.clone())
            .unwrap_or_default();
        for old_id in &old_block_ids {
            self.dependencies.remove_block(old_id);
            self.versions.remove(old_id);
        }
        self.store.remove_many(&old_block_ids);

        // Store new blocks
        let block_ids: Vec<String> = all_blocks.iter().map(|b| b.id.clone()).collect();
        let exchange_tokens: u32 = all_blocks.iter().map(|b| b.tokens).sum();

        self.store.insert_many(all_blocks);

        // Replace session tracking (not accumulate)
        self.sessions.update(&session_id, |s| {
            s.block_ids = block_ids.clone();
            s.total_tokens = exchange_tokens;
            s.exchange_count += 1;
        });

        // Run classification pipeline on all session blocks
        let session_block_ids = self
            .sessions
            .get(&session_id)
            .map(|s| s.block_ids.clone())
            .unwrap_or_default();
        let mut session_blocks = self.store.get_many(&session_block_ids);

        let classification = classify(&mut session_blocks, &self.pipeline_config);

        // Apply zone changes back to store
        for block in &session_blocks {
            self.store.update(&block.id, |b| {
                b.zone = block.zone.clone();
                b.usage_heat = block.usage_heat;
            });
        }

        // Build dependency graph
        let edges = build_dependencies(&session_blocks);
        for edge in edges {
            self.dependencies.add_edge(edge);
        }

        // Update reference counts
        for block_id in &block_ids {
            let deps = self.dependencies.dependents_of(block_id);
            let ref_count = deps.len() as u32;
            self.store.update(block_id, |b| {
                b.reference_count = ref_count;
            });
        }

        // Log action
        let record = action_log::new_record(
            ActionActor::Pipeline,
            ActionKind::Ingest,
            block_ids,
            format!(
                "Ingested exchange from {provider}/{model} [{source}/{}]",
                thread_id.unwrap_or("default")
            ),
        );
        self.action_log.record(record);

        // Persist to SQLite (background, best-effort)
        self.persist_session(&session_id);

        // Emit context updated event
        let total_tokens = self.store.total_tokens();
        let block_count = self.store.count() as u32;
        self.emit_context_updated(block_count, total_tokens);

        debug!(
            "Ingested exchange: session={}, source={}, thread={}, blocks={}, tokens={}, alert={:?}",
            session_id,
            source,
            thread_id.unwrap_or("default"),
            session_blocks.len(),
            total_tokens,
            classification.alert_level
        );

        IngestResult {
            session_id,
            block_count: session_blocks.len(),
            total_tokens,
            alert_level: classification.alert_level,
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

    /// Budget status for the active session.
    pub fn budget_status(&self) -> BudgetStatus {
        let session = self.sessions.active();
        let (used, limit) = match &session {
            Some(s) => (s.total_tokens, s.token_budget),
            None => (self.store.total_tokens(), 200_000),
        };
        budget::budget_status(used, limit, &BudgetConfig::default())
    }

    /// Current sidekick compression settings.
    pub fn compression_settings(&self) -> CompressionSettings {
        self.compression_settings
            .lock()
            .expect("compression_settings lock")
            .clone()
    }

    /// Update sidekick compression settings.
    pub fn set_compression_settings(&self, settings: CompressionSettings) {
        let normalized = settings.normalized();
        let mut guard = self
            .compression_settings
            .lock()
            .expect("compression_settings lock");
        *guard = normalized;
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
        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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

        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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

        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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

        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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

        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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

        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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

        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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

        self.emit_context_updated(self.store.count() as u32, self.store.total_tokens());
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
                self.sessions.switch_to(&existing_id);
                return existing_id;
            }
            self.session_identity_index.remove(&identity_key);
        }

        let session_id = self.create_session(provider, model, source, thread_id);
        self.session_identity_index
            .insert(identity_key, session_id.clone());
        session_id
    }

    /// Persist the currently active session and its blocks.
    fn persist_active_session(&self) {
        if let Some(session_id) = self.sessions.active_id() {
            self.persist_session(&session_id);
        }
    }

    /// Keep active session block list/token totals aligned with the current in-memory store.
    fn refresh_active_session_totals(&self) {
        let Some(session_id) = self.sessions.active_id() else {
            return;
        };
        let Some(session) = self.sessions.get(&session_id) else {
            return;
        };

        let block_ids: Vec<String> = session
            .block_ids
            .into_iter()
            .filter(|id| self.store.contains(id))
            .collect();
        let total_tokens: u32 = block_ids
            .iter()
            .filter_map(|id| self.store.get(id).map(|b| b.tokens))
            .sum();

        self.sessions.update(&session_id, |s| {
            s.block_ids = block_ids;
            s.total_tokens = total_tokens;
        });
    }

    /// Persist session data to SQLite.
    fn persist_session(&self, session_id: &str) {
        let Some(ref db) = self.db else { return };

        if let Some(session) = self.sessions.get(session_id) {
            if let Err(e) = db.save_session(&session) {
                warn!("Failed to persist session: {e}");
            }

            let blocks = self.store.get_many(&session.block_ids);
            if let Err(e) = db.save_blocks(&blocks, Some(session_id)) {
                warn!("Failed to persist blocks: {e}");
            }
        }
    }

    /// Emit a ContextUpdated event to the frontend.
    fn emit_context_updated(&self, block_count: u32, total_tokens: u32) {
        if let Some(ref dispatcher) = self.dispatcher {
            dispatcher.emit(&ApertureEvent::ContextUpdated {
                block_count,
                total_tokens,
            });
        }
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
mod tests {
    use super::*;
    use block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use compression::CompressionBackendKind;
    use types::{BuiltInZone, CompressionLevel, PinPosition, Role, Zone};

    fn make_block(id: &str, role: Role, content: &str) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: content.to_string(),
            tokens: (content.len() as u32) / 4,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: content.to_string(),
                    tokens: (content.len() as u32) / 4,
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
                provider: "anthropic".to_string(),
                turn_index: 0,
                tool_name: None,
                file_paths: vec![],
            },
        }
    }

    fn make_block_with_turn(id: &str, role: Role, content: &str, turn_index: u32) -> Block {
        let mut block = make_block(id, role, content);
        block.metadata.turn_index = turn_index;
        block
    }

    #[test]
    fn test_ingest_replaces_blocks_not_accumulates() {
        let engine = ContextEngine::new_in_memory(None);

        // First ingest: 2 request blocks + 1 response block
        let req1 = vec![
            make_block("r1a", Role::System, "You are a helpful assistant"),
            make_block("r1b", Role::User, "Hello"),
        ];
        let resp1 = vec![make_block("a1", Role::Assistant, "Hi there!")];

        let result1 = engine.ingest("anthropic", "claude", "proxy", None, req1, resp1);
        assert_eq!(result1.block_count, 3);
        assert_eq!(engine.store.count(), 3);

        // Second ingest: full conversation history (new UUIDs) + new response
        let req2 = vec![
            make_block("r2a", Role::System, "You are a helpful assistant"),
            make_block("r2b", Role::User, "Hello"),
            make_block("r2c", Role::Assistant, "Hi there!"),
            make_block("r2d", Role::User, "How are you?"),
        ];
        let resp2 = vec![make_block("a2", Role::Assistant, "I'm doing well!")];

        let result2 = engine.ingest("anthropic", "claude", "proxy", None, req2, resp2);
        assert_eq!(result2.block_count, 5);
        // Critical: store should have exactly 5 blocks (replaced, not 3+5=8)
        assert_eq!(engine.store.count(), 5);

        // Old blocks should be gone
        assert!(engine.store.get("r1a").is_none());
        assert!(engine.store.get("r1b").is_none());
        assert!(engine.store.get("a1").is_none());

        // New blocks should be present
        assert!(engine.store.get("r2a").is_some());
        assert!(engine.store.get("a2").is_some());
    }

    #[test]
    fn test_ingest_session_tokens_reflect_latest() {
        let engine = ContextEngine::new_in_memory(None);

        let req1 = vec![make_block("r1", Role::User, "short")];
        let resp1 = vec![make_block("a1", Role::Assistant, "short reply")];
        engine.ingest("anthropic", "claude", "proxy", None, req1, resp1);

        let session1 = engine.active_session().unwrap();
        let tokens_after_first = session1.total_tokens;

        // Second ingest — different token count
        let req2 = vec![
            make_block("r2a", Role::User, "short"),
            make_block(
                "r2b",
                Role::User,
                "this is a much longer message with more tokens",
            ),
        ];
        let resp2 = vec![make_block(
            "a2",
            Role::Assistant,
            "this is also a longer response with many more tokens than before",
        )];
        engine.ingest("anthropic", "claude", "proxy", None, req2, resp2);

        let session2 = engine.active_session().unwrap();
        // Tokens should reflect the latest ingest, not accumulate
        assert_ne!(
            session2.total_tokens,
            tokens_after_first + session2.total_tokens
        );
        assert_eq!(session2.exchange_count, 2);
    }

    #[test]
    fn test_ingest_normalizes_response_turn_to_latest_request_turn() {
        let engine = ContextEngine::new_in_memory(None);

        let request_blocks = vec![
            make_block_with_turn("u0", Role::User, "turn 0", 0),
            make_block_with_turn("a0", Role::Assistant, "turn 1", 1),
            make_block_with_turn("u1", Role::User, "turn 2", 2),
            make_block_with_turn("a1", Role::Assistant, "turn 3", 3),
            make_block_with_turn("u2", Role::User, "turn 4", 4),
            make_block_with_turn("a2", Role::Assistant, "turn 5", 5),
        ];
        // Response parsers emit turn_index=0; ingest should normalize this to latest+1.
        let response_blocks = vec![make_block_with_turn(
            "latest-assistant",
            Role::Assistant,
            "newest reply",
            0,
        )];

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            request_blocks,
            response_blocks,
        );

        let latest = engine
            .block("latest-assistant")
            .expect("response block exists");
        assert_eq!(
            latest.metadata.turn_index, 6,
            "Response block should be shifted to latest request turn + 1"
        );
        assert_eq!(
            latest.zone,
            Zone::BuiltIn(BuiltInZone::Recency),
            "Newest response must stay in recency"
        );
    }

    #[test]
    fn test_ingest_clears_stale_dependencies_when_replacing_session_blocks() {
        let engine = ContextEngine::new_in_memory(None);

        // First exchange creates a conversation dependency edge.
        engine.ingest(
            "anthropic",
            "claude",
            "proxy",
            None,
            vec![make_block_with_turn("u1", Role::User, "hello", 0)],
            vec![make_block_with_turn("a1", Role::Assistant, "hi", 1)],
        );
        assert_eq!(engine.dependencies.edge_count(), 1);

        // Next exchange has only one block; stale edge should be removed.
        engine.ingest(
            "anthropic",
            "claude",
            "proxy",
            None,
            vec![make_block_with_turn("u2", Role::User, "fresh", 2)],
            vec![],
        );
        assert_eq!(
            engine.dependencies.edge_count(),
            0,
            "Dependency graph should not retain edges to replaced blocks"
        );
    }

    #[test]
    fn test_undo_restores_previous_content_after_single_edit() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "original")],
            vec![],
        );

        let decision = engine
            .update_content("u1", "edited once", "claude-sonnet", false)
            .expect("update should succeed");
        assert!(matches!(decision, PolicyDecision::Allow));
        assert_eq!(engine.block("u1").unwrap().content, "edited once");

        engine.undo_block("u1").expect("undo should succeed");
        assert_eq!(engine.block("u1").unwrap().content, "original");
    }

    #[test]
    fn test_ensure_session_distinguishes_model_within_provider() {
        let engine = ContextEngine::new_in_memory(None);

        let first = engine.ingest(
            "openai",
            "gpt-4o",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );
        let second = engine.ingest(
            "openai",
            "gpt-5",
            "proxy",
            None,
            vec![make_block("u2", Role::User, "hi")],
            vec![],
        );

        assert_ne!(
            first.session_id, second.session_id,
            "Different models should not be merged into one session"
        );
        assert_eq!(engine.list_sessions().len(), 2);
    }

    #[test]
    fn test_ensure_session_distinguishes_thread_identity_within_provider_model() {
        let engine = ContextEngine::new_in_memory(None);

        let first = engine.ingest(
            "openai",
            "codex-subscription",
            "direct_cli_bridge",
            Some("thread-a"),
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );
        let second = engine.ingest(
            "openai",
            "codex-subscription",
            "direct_cli_bridge",
            Some("thread-b"),
            vec![make_block("u2", Role::User, "hi")],
            vec![],
        );

        assert_ne!(
            first.session_id, second.session_id,
            "Different direct thread IDs should not merge into one session"
        );
        assert_eq!(engine.list_sessions().len(), 2);
    }

    #[test]
    fn test_ensure_session_reuses_session_for_same_source_and_thread_identity() {
        let engine = ContextEngine::new_in_memory(None);

        let first = engine.ingest(
            "openai",
            "codex-subscription",
            "direct_cli_bridge",
            Some("thread-a"),
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );
        let second = engine.ingest(
            "openai",
            "codex-subscription",
            "direct_cli_bridge",
            Some("thread-a"),
            vec![make_block("u2", Role::User, "hi")],
            vec![],
        );

        assert_eq!(
            first.session_id, second.session_id,
            "Same source/thread identity should reuse the same session"
        );
        assert_eq!(engine.list_sessions().len(), 1);
    }

    #[test]
    fn test_session_info_exposes_source_and_thread_identity() {
        let engine = ContextEngine::new_in_memory(None);

        let result = engine.ingest(
            "openai",
            "codex-subscription",
            "direct_cli_bridge",
            Some("thread-a"),
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );

        let session = engine
            .list_sessions()
            .into_iter()
            .find(|item| item.id == result.session_id)
            .expect("session should exist");
        assert_eq!(session.source, "direct_cli_bridge");
        assert_eq!(session.thread_identity.as_deref(), Some("thread-a"));
    }

    #[test]
    fn test_update_and_remove_keep_session_totals_in_sync() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![
                make_block("u1", Role::User, "short"),
                make_block("u2", Role::User, "tiny"),
            ],
            vec![],
        );

        let before = engine.active_session().expect("session exists");
        assert_eq!(before.block_count, 2);

        engine
            .update_content(
                "u1",
                "this message is much longer than before",
                "claude-sonnet",
                false,
            )
            .expect("update should succeed");

        let after_edit = engine.active_session().expect("session exists");
        assert!(
            after_edit.total_tokens > before.total_tokens,
            "session total tokens should reflect edited content token changes"
        );

        engine
            .remove_block("u2", true)
            .expect("remove should succeed with confirmation");

        let after_remove = engine.active_session().expect("session exists");
        assert_eq!(after_remove.block_count, 1);
        assert_eq!(after_remove.total_tokens, engine.store.total_tokens());
    }

    #[test]
    fn test_noop_update_does_not_record_version_or_action() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "same content")],
            vec![],
        );

        let action_count_before = engine.action_log.count();
        let versions_before = engine.block_versions("u1").len();
        let decision = engine
            .update_content("u1", "same content", "claude-sonnet", false)
            .expect("no-op update should succeed");

        assert!(matches!(decision, PolicyDecision::Allow));
        assert_eq!(
            engine.action_log.count(),
            action_count_before,
            "no-op edit should not add an action log entry"
        );
        assert_eq!(
            engine.block_versions("u1").len(),
            versions_before,
            "no-op edit should not add a version snapshot"
        );
    }

    #[test]
    fn test_noop_move_pin_and_compress_do_not_log_actions() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "stable block")],
            vec![],
        );

        let zone = engine.block("u1").expect("block exists").zone;
        let action_count_before = engine.action_log.count();

        let move_decision = engine
            .move_block("u1", zone, false)
            .expect("no-op move should succeed");
        assert!(matches!(move_decision, PolicyDecision::Allow));

        let pin_decision = engine
            .pin_block("u1", None)
            .expect("no-op pin should succeed");
        assert!(matches!(pin_decision, PolicyDecision::Allow));

        let compress_decision = engine
            .compress_block("u1", CompressionLevel::Original, false)
            .expect("no-op compression should succeed");
        assert!(matches!(compress_decision, PolicyDecision::Allow));

        assert_eq!(
            engine.action_log.count(),
            action_count_before,
            "no-op mutations should not add action log entries"
        );
    }

    #[test]
    fn test_bulk_remove_updates_session_block_ids() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![
                make_block("u1", Role::User, "one"),
                make_block("u2", Role::User, "two"),
                make_block("u3", Role::User, "three"),
            ],
            vec![],
        );

        let ids = vec!["u1".to_string(), "u3".to_string()];
        let decision = engine
            .bulk_remove(&ids, true)
            .expect("bulk remove should succeed");
        assert!(matches!(decision, PolicyDecision::Allow));

        let session_info = engine.active_session().expect("session exists");
        assert_eq!(session_info.block_count, 1);
        assert_eq!(session_info.total_tokens, engine.store.total_tokens());

        let session = engine
            .sessions
            .active()
            .expect("active session should exist");
        assert_eq!(session.block_ids, vec!["u2".to_string()]);
    }

    #[test]
    fn test_remove_requires_confirmation_for_pinned_blocks() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "important context")],
            vec![],
        );
        engine
            .pin_block("u1", Some(PinPosition::Top))
            .expect("pin should succeed");

        let first = engine
            .remove_block("u1", false)
            .expect("policy check should return decision");
        assert!(matches!(first, PolicyDecision::RequireConfirmation { .. }));
        assert!(
            engine.block("u1").is_some(),
            "block should remain when unconfirmed"
        );

        let second = engine
            .remove_block("u1", true)
            .expect("confirmed remove should succeed");
        assert!(matches!(second, PolicyDecision::Allow));
        assert!(engine.block("u1").is_none());
    }

    #[test]
    fn test_bulk_remove_requires_confirmation_for_large_set() {
        let engine = ContextEngine::new_in_memory(None);

        let mut req = Vec::new();
        for i in 0..6 {
            req.push(make_block(&format!("u{i}"), Role::User, "x"));
        }
        engine.ingest("anthropic", "claude-sonnet", "proxy", None, req, vec![]);

        let ids: Vec<String> = (0..6).map(|i| format!("u{i}")).collect();
        let first = engine
            .bulk_remove(&ids, false)
            .expect("bulk policy check should return decision");
        assert!(matches!(first, PolicyDecision::RequireConfirmation { .. }));
        assert_eq!(engine.store.count(), 6);

        let second = engine
            .bulk_remove(&ids, true)
            .expect("confirmed bulk remove should succeed");
        assert!(matches!(second, PolicyDecision::Allow));
        assert_eq!(engine.store.count(), 0);
    }

    // ── System-driven block mutation tests ────────────────

    #[test]
    fn test_move_block_internal_changes_zone() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );

        // Initially in Recency (assigned by ingest zone logic)
        let original_zone = engine.block("u1").unwrap().zone;

        engine.move_block_internal("u1", Zone::BuiltIn(BuiltInZone::Primacy));
        let updated = engine.block("u1").unwrap();
        assert_eq!(updated.zone, Zone::BuiltIn(BuiltInZone::Primacy));
        assert_ne!(updated.zone, original_zone);
    }

    #[test]
    fn test_move_block_internal_noop_same_zone() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );

        let zone = engine.block("u1").unwrap().zone;
        // Moving to the same zone should be a no-op
        engine.move_block_internal("u1", zone.clone());
        assert_eq!(engine.block("u1").unwrap().zone, zone);
    }

    #[test]
    fn test_move_block_internal_unknown_id_ignored() {
        let engine = ContextEngine::new_in_memory(None);
        // Should not panic
        engine.move_block_internal("nonexistent", Zone::BuiltIn(BuiltInZone::Primacy));
    }

    #[test]
    fn test_set_pin_internal_pins_block() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );

        assert!(engine.block("u1").unwrap().pinned.is_none());
        engine.set_pin_internal("u1", Some(PinPosition::Top));
        assert_eq!(engine.block("u1").unwrap().pinned, Some(PinPosition::Top));
    }

    #[test]
    fn test_set_pin_internal_unpins_block() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "hello")],
            vec![],
        );

        engine.set_pin_internal("u1", Some(PinPosition::Top));
        assert_eq!(engine.block("u1").unwrap().pinned, Some(PinPosition::Top));

        engine.set_pin_internal("u1", None);
        assert!(engine.block("u1").unwrap().pinned.is_none());
    }

    #[test]
    fn test_set_pin_internal_unknown_id_ignored() {
        let engine = ContextEngine::new_in_memory(None);
        engine.set_pin_internal("nonexistent", Some(PinPosition::Top));
    }

    #[test]
    fn test_internal_mutations_visible_in_session_blocks() {
        let engine = ContextEngine::new_in_memory(None);

        engine.ingest(
            "anthropic",
            "claude-sonnet",
            "proxy",
            None,
            vec![
                make_block("u1", Role::User, "hello"),
                make_block("u2", Role::User, "world"),
            ],
            vec![],
        );

        engine.move_block_internal("u1", Zone::BuiltIn(BuiltInZone::Primacy));
        engine.set_pin_internal("u2", Some(PinPosition::Top));

        let blocks = engine.active_session_blocks();
        let u1 = blocks.iter().find(|b| b.id == "u1").unwrap();
        let u2 = blocks.iter().find(|b| b.id == "u2").unwrap();
        assert_eq!(u1.zone, Zone::BuiltIn(BuiltInZone::Primacy));
        assert_eq!(u2.pinned, Some(PinPosition::Top));
    }

    #[test]
    fn test_archive_block_internal_removes_block_and_updates_totals() {
        let engine = ContextEngine::new_in_memory(None);
        engine.ingest(
            "openai",
            "gpt-4.1",
            "proxy",
            None,
            vec![
                make_block("u1", Role::User, "alpha"),
                make_block("u2", Role::User, "beta"),
            ],
            vec![],
        );

        let before = engine.active_session().expect("active session");
        assert_eq!(before.block_count, 2);
        engine.archive_block_internal("u1");

        assert!(engine.block("u1").is_none());
        let after = engine.active_session().expect("active session");
        assert_eq!(after.block_count, 1);
        assert_eq!(after.total_tokens, engine.store.total_tokens());
    }

    #[test]
    fn test_apply_compression_summary_internal_updates_block_state() {
        let engine = ContextEngine::new_in_memory(None);
        engine.ingest(
            "openai",
            "gpt-4.1",
            "proxy",
            None,
            vec![make_block(
                "u1",
                Role::User,
                "long original content for compression testing",
            )],
            vec![],
        );

        engine.apply_compression_summary_internal("u1", "short summary");
        let block = engine.block("u1").expect("block exists");
        assert_eq!(block.content, "short summary");
        assert_eq!(block.compression_level, CompressionLevel::Summarized);
        assert_eq!(
            block
                .compressed_versions
                .summarized
                .as_ref()
                .map(|v| v.content.as_str()),
            Some("short summary")
        );
    }

    #[test]
    fn test_restore_original_internal_rehydrates_original_content() {
        let engine = ContextEngine::new_in_memory(None);
        engine.ingest(
            "openai",
            "gpt-4.1",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "original content")],
            vec![],
        );

        engine.apply_compression_summary_internal("u1", "compressed");
        engine.restore_original_internal("u1");

        let block = engine.block("u1").expect("block exists");
        assert_eq!(block.content, "original content");
        assert_eq!(block.compression_level, CompressionLevel::Original);
    }

    #[test]
    fn test_update_content_internal_resets_compression_versions() {
        let engine = ContextEngine::new_in_memory(None);
        engine.ingest(
            "openai",
            "gpt-4.1",
            "proxy",
            None,
            vec![make_block("u1", Role::User, "old content")],
            vec![],
        );

        engine.apply_compression_summary_internal("u1", "old summary");
        engine.update_content_internal("u1", "new canonical content");

        let block = engine.block("u1").expect("block exists");
        assert_eq!(block.content, "new canonical content");
        assert_eq!(block.compression_level, CompressionLevel::Original);
        assert_eq!(
            block.compressed_versions.original.content,
            "new canonical content"
        );
        assert!(block.compressed_versions.summarized.is_none());
    }

    #[test]
    fn test_compression_settings_default_available_from_engine() {
        let engine = ContextEngine::new_in_memory(None);
        let settings = engine.compression_settings();
        assert_eq!(settings.backend, CompressionBackendKind::Auto);
        assert_eq!(settings.timeout_ms, 12_000);
        assert_eq!(settings.max_tokens, 512);
    }

    #[test]
    fn test_set_compression_settings_normalizes_values() {
        let engine = ContextEngine::new_in_memory(None);
        engine.set_compression_settings(compression::CompressionSettings {
            backend: CompressionBackendKind::OpenRouter,
            model: Some("  custom-model  ".to_string()),
            timeout_ms: 100,
            max_tokens: 40_000,
            openrouter_base_url: Some(" https://openrouter.ai/api/v1 ".to_string()),
            openrouter_api_key_env: Some("   ".to_string()),
        });

        let settings = engine.compression_settings();
        assert_eq!(settings.backend, CompressionBackendKind::OpenRouter);
        assert_eq!(settings.model.as_deref(), Some("custom-model"));
        assert_eq!(settings.timeout_ms, 500);
        assert_eq!(settings.max_tokens, 8_192);
        assert_eq!(
            settings.openrouter_base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(settings.openrouter_api_key_env.is_none());
    }
}
