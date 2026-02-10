//! Structured action log for audit trail and undo support.
//!
//! Every mutation to context blocks is recorded with enough information
//! to understand what happened, why, and potentially undo it.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Maximum number of action records to retain.
const MAX_RECORDS: usize = 500;

/// Who initiated the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionActor {
    /// User-initiated via UI.
    User,
    /// Proxy hot patch.
    HotPatch,
    /// Engine pipeline (auto-classification, zone assignment).
    Pipeline,
    /// Compression system.
    Compression,
    /// Policy engine (auto-cleanup, budget enforcement).
    Policy,
}

/// What type of action was performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Block content was edited.
    EditContent,
    /// Block was moved to a different zone.
    MoveZone,
    /// Block was compressed.
    Compress,
    /// Block was removed.
    Remove,
    /// Block was pinned/unpinned.
    Pin,
    /// Block's role was changed.
    ChangeRole,
    /// Multiple blocks were added (ingestion).
    Ingest,
    /// Multiple blocks were removed (bulk).
    BulkRemove,
    /// Session was cleared.
    ClearSession,
}

/// The outcome of an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionResult {
    /// Action succeeded.
    Success,
    /// Action was blocked by policy.
    Denied { reason: String },
    /// Action required confirmation and was pending.
    PendingConfirmation,
}

/// A record of a single action performed on context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    /// Unique identifier for this action.
    pub id: String,
    /// When the action occurred (ISO 8601).
    pub timestamp: String,
    /// Who initiated the action.
    pub actor: ActionActor,
    /// What type of action was performed.
    pub kind: ActionKind,
    /// Which blocks were affected.
    pub target_block_ids: Vec<String>,
    /// Session ID if applicable.
    pub session_id: Option<String>,
    /// The outcome of the action.
    pub result: ActionResult,
    /// Whether this action can be undone.
    pub undoable: bool,
    /// Snapshot of previous state for undo (serialized).
    /// Only populated for undoable actions.
    pub undo_data: Option<String>,
    /// Human-readable description of what happened.
    pub description: String,
}

/// Thread-safe action log.
pub struct ActionLog {
    records: Mutex<Vec<ActionRecord>>,
}

impl ActionLog {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }

    /// Record an action.
    pub fn record(&self, record: ActionRecord) {
        let mut records = self.records.lock().expect("action_log lock poisoned");
        records.push(record);

        // Evict oldest if over limit
        if records.len() > MAX_RECORDS {
            let excess = records.len() - MAX_RECORDS;
            records.drain(..excess);
        }
    }

    /// Get all records (oldest first).
    pub fn all(&self) -> Vec<ActionRecord> {
        self.records
            .lock()
            .expect("action_log lock poisoned")
            .clone()
    }

    /// Get the N most recent records.
    pub fn recent(&self, n: usize) -> Vec<ActionRecord> {
        let records = self.records.lock().expect("action_log lock poisoned");
        let start = records.len().saturating_sub(n);
        records[start..].to_vec()
    }

    /// Get a specific record by ID.
    pub fn get(&self, id: &str) -> Option<ActionRecord> {
        self.records
            .lock()
            .expect("action_log lock poisoned")
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// Get the most recent undoable action for a block.
    pub fn last_undoable_for(&self, block_id: &str) -> Option<ActionRecord> {
        self.records
            .lock()
            .expect("action_log lock poisoned")
            .iter()
            .rev()
            .find(|r| {
                r.undoable
                    && r.undo_data.is_some()
                    && r.result == ActionResult::Success
                    && r.target_block_ids.contains(&block_id.to_string())
            })
            .cloned()
    }

    /// Total number of recorded actions.
    pub fn count(&self) -> usize {
        self.records.lock().expect("action_log lock poisoned").len()
    }

    /// Clear all records.
    pub fn clear(&self) {
        self.records
            .lock()
            .expect("action_log lock poisoned")
            .clear();
    }
}

impl Default for ActionLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to create a new action record with a generated ID.
pub fn new_record(
    actor: ActionActor,
    kind: ActionKind,
    target_block_ids: Vec<String>,
    description: String,
) -> ActionRecord {
    ActionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: iso_now(),
        actor,
        kind,
        target_block_ids,
        session_id: None,
        result: ActionResult::Success,
        undoable: false,
        undo_data: None,
        description,
    }
}

fn iso_now() -> String {
    crate::util::iso_now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_retrieve() {
        let log = ActionLog::new();

        let record = new_record(
            ActionActor::User,
            ActionKind::EditContent,
            vec!["b1".into()],
            "Edited block b1 content".into(),
        );
        log.record(record);

        assert_eq!(log.count(), 1);
        let all = log.all();
        assert_eq!(all[0].actor, ActionActor::User);
        assert_eq!(all[0].kind, ActionKind::EditContent);
    }

    #[test]
    fn test_recent_records() {
        let log = ActionLog::new();

        for i in 0..10 {
            let record = new_record(
                ActionActor::Pipeline,
                ActionKind::MoveZone,
                vec![format!("b{i}")],
                format!("Moved block b{i}"),
            );
            log.record(record);
        }

        let recent = log.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].description.contains("b7"));
    }

    #[test]
    fn test_undoable_lookup() {
        let log = ActionLog::new();

        let mut r1 = new_record(
            ActionActor::User,
            ActionKind::EditContent,
            vec!["b1".into()],
            "First edit".into(),
        );
        r1.undoable = true;
        r1.undo_data = Some(r#"{"content":"original"}"#.into());
        log.record(r1);

        let mut r2 = new_record(
            ActionActor::User,
            ActionKind::MoveZone,
            vec!["b2".into()],
            "Moved b2".into(),
        );
        r2.undoable = true;
        r2.undo_data = Some(r#"{"zone":"middle"}"#.into());
        log.record(r2);

        let undoable = log.last_undoable_for("b1").unwrap();
        assert_eq!(undoable.description, "First edit");

        assert!(log.last_undoable_for("nonexistent").is_none());
    }

    #[test]
    fn test_eviction() {
        let log = ActionLog::new();

        for i in 0..(MAX_RECORDS + 50) {
            let record = new_record(
                ActionActor::Pipeline,
                ActionKind::Ingest,
                vec![format!("b{i}")],
                format!("Ingest {i}"),
            );
            log.record(record);
        }

        assert_eq!(log.count(), MAX_RECORDS);
    }
}
