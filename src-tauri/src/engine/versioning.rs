//! Block version tracking.
//!
//! Tracks the last N edits per block for undo support.
//! Foundation for Phase 11's advanced versioning UX.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

/// Maximum number of versions to retain per block.
const MAX_VERSIONS: usize = 5;

/// What triggered the edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditSource {
    /// User manually edited content.
    User,
    /// Automatic rule applied a change.
    AutoRule,
    /// Hot patch from the proxy.
    HotPatch,
    /// Compression changed the content.
    Compression,
    /// Engine pipeline modified the block.
    Pipeline,
}

/// A single version of a block's content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockVersion {
    pub version: u32,
    pub content: String,
    pub tokens: u32,
    pub edited_at: String,
    pub source: EditSource,
}

/// Version history for a single block.
#[derive(Debug, Clone)]
pub struct VersionHistory {
    /// Versions ordered newest-first.
    pub versions: Vec<BlockVersion>,
    /// Next version number.
    next_version: u32,
}

impl VersionHistory {
    fn new() -> Self {
        Self {
            versions: Vec::new(),
            next_version: 1,
        }
    }

    /// Record a new version. Evicts oldest if over limit.
    fn push(&mut self, content: String, tokens: u32, source: EditSource) -> u32 {
        let version = self.next_version;
        self.next_version += 1;

        self.versions.insert(
            0,
            BlockVersion {
                version,
                content,
                tokens,
                edited_at: iso_now(),
                source,
            },
        );

        // Evict oldest if over limit
        if self.versions.len() > MAX_VERSIONS {
            self.versions.truncate(MAX_VERSIONS);
        }

        version
    }

    /// Get the most recent version (current state before this edit).
    fn latest(&self) -> Option<&BlockVersion> {
        self.versions.first()
    }

    /// Get a specific version.
    fn get_version(&self, version: u32) -> Option<&BlockVersion> {
        self.versions.iter().find(|v| v.version == version)
    }
}

/// Concurrent version store for all blocks.
pub struct VersionStore {
    histories: DashMap<String, VersionHistory>,
}

impl VersionStore {
    pub fn new() -> Self {
        Self {
            histories: DashMap::new(),
        }
    }

    /// Record a new version for a block.
    /// Returns the version number.
    pub fn record(&self, block_id: &str, content: String, tokens: u32, source: EditSource) -> u32 {
        let mut entry = self
            .histories
            .entry(block_id.to_string())
            .or_insert_with(VersionHistory::new);
        entry.push(content, tokens, source)
    }

    /// Get the full version history for a block.
    pub fn history(&self, block_id: &str) -> Vec<BlockVersion> {
        self.histories
            .get(block_id)
            .map(|h| h.versions.clone())
            .unwrap_or_default()
    }

    /// Get the latest version for a block.
    pub fn latest(&self, block_id: &str) -> Option<BlockVersion> {
        self.histories
            .get(block_id)
            .and_then(|h| h.latest().cloned())
    }

    /// Get a specific version of a block.
    pub fn get_version(&self, block_id: &str, version: u32) -> Option<BlockVersion> {
        self.histories
            .get(block_id)
            .and_then(|h| h.get_version(version).cloned())
    }

    /// Pop and return the most recent prior state to undo to.
    ///
    /// Versions are recorded as "pre-edit snapshots", newest first.
    /// Undo should therefore restore and consume `versions[0]`.
    pub fn undo_content(&self, block_id: &str) -> Option<(String, u32)> {
        self.histories.get_mut(block_id).and_then(|mut h| {
            if h.versions.is_empty() {
                None
            } else {
                let restored = h.versions.remove(0);
                Some((restored.content, restored.tokens))
            }
        })
    }

    /// Number of blocks with version history.
    pub fn tracked_block_count(&self) -> usize {
        self.histories.len()
    }

    /// Remove version history for a block.
    pub fn remove(&self, block_id: &str) {
        self.histories.remove(block_id);
    }

    /// Clear all version history.
    pub fn clear(&self) {
        self.histories.clear();
    }
}

impl Default for VersionStore {
    fn default() -> Self {
        Self::new()
    }
}

fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Delegate to session's chrono_now equivalent
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_retrieve() {
        let store = VersionStore::new();

        let v1 = store.record("b1", "original content".into(), 10, EditSource::User);
        assert_eq!(v1, 1);

        let v2 = store.record("b1", "edited content".into(), 12, EditSource::User);
        assert_eq!(v2, 2);

        let history = store.history("b1");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].version, 2); // newest first
        assert_eq!(history[0].content, "edited content");
    }

    #[test]
    fn test_version_limit() {
        let store = VersionStore::new();

        for i in 0..8 {
            store.record("b1", format!("v{i}"), (i + 1) * 10, EditSource::User);
        }

        let history = store.history("b1");
        assert_eq!(history.len(), MAX_VERSIONS); // capped at 5
        assert_eq!(history[0].version, 8); // newest
    }

    #[test]
    fn test_undo_content() {
        let store = VersionStore::new();

        store.record("b1", "original".into(), 10, EditSource::User);
        store.record("b1", "edited".into(), 12, EditSource::User);

        let (content, tokens) = store.undo_content("b1").unwrap();
        assert_eq!(content, "edited");
        assert_eq!(tokens, 12);

        let (content2, tokens2) = store.undo_content("b1").unwrap();
        assert_eq!(content2, "original");
        assert_eq!(tokens2, 10);
    }

    #[test]
    fn test_undo_no_previous() {
        let store = VersionStore::new();
        store.record("b1", "only version".into(), 10, EditSource::User);

        let undone = store.undo_content("b1");
        assert!(undone.is_some());
        assert!(store.undo_content("b1").is_none());
    }

    #[test]
    fn test_empty_history() {
        let store = VersionStore::new();
        assert!(store.history("nonexistent").is_empty());
        assert!(store.latest("nonexistent").is_none());
    }

    #[test]
    fn test_different_blocks_independent() {
        let store = VersionStore::new();

        store.record("b1", "block1 content".into(), 10, EditSource::User);
        store.record("b2", "block2 content".into(), 20, EditSource::HotPatch);

        assert_eq!(store.tracked_block_count(), 2);
        assert_eq!(store.history("b1").len(), 1);
        assert_eq!(store.history("b2").len(), 1);
        assert_eq!(store.history("b2")[0].source, EditSource::HotPatch);
    }
}
