//! In-memory block storage.
//!
//! `BlockStore` is the hot-path data structure for the current session's blocks.
//! It uses `DashMap` for lock-free concurrent reads and fine-grained write locking.

use std::collections::HashMap;

use dashmap::DashMap;

use super::block::Block;
use super::types::{Role, Zone};

/// Concurrent in-memory block store.
pub struct BlockStore {
    blocks: DashMap<String, Block>,
}

impl BlockStore {
    pub fn new() -> Self {
        Self {
            blocks: DashMap::new(),
        }
    }

    // ── CRUD ──────────────────────────────────────────────────────

    /// Insert a block, returning its ID.
    pub fn insert(&self, block: Block) -> String {
        let id = block.id.clone();
        self.blocks.insert(id.clone(), block);
        id
    }

    /// Get a block by ID (cloned).
    pub fn get(&self, id: &str) -> Option<Block> {
        self.blocks.get(id).map(|r| r.value().clone())
    }

    /// Mutate a block in-place. Returns `true` if the block existed.
    pub fn update<F: FnOnce(&mut Block)>(&self, id: &str, f: F) -> bool {
        if let Some(mut entry) = self.blocks.get_mut(id) {
            f(entry.value_mut());
            true
        } else {
            false
        }
    }

    /// Remove a block, returning it if it existed.
    pub fn remove(&self, id: &str) -> Option<Block> {
        self.blocks.remove(id).map(|(_, b)| b)
    }

    /// Check whether a block exists.
    pub fn contains(&self, id: &str) -> bool {
        self.blocks.contains_key(id)
    }

    // ── Batch ─────────────────────────────────────────────────────

    /// Insert multiple blocks at once.
    pub fn insert_many(&self, blocks: Vec<Block>) {
        for block in blocks {
            self.blocks.insert(block.id.clone(), block);
        }
    }

    /// Get multiple blocks by ID.
    pub fn get_many(&self, ids: &[String]) -> Vec<Block> {
        ids.iter()
            .filter_map(|id| self.blocks.get(id.as_str()).map(|r| r.value().clone()))
            .collect()
    }

    /// Remove multiple blocks, returning those that existed.
    pub fn remove_many(&self, ids: &[String]) -> Vec<Block> {
        ids.iter()
            .filter_map(|id| self.blocks.remove(id.as_str()).map(|(_, b)| b))
            .collect()
    }

    // ── Queries ───────────────────────────────────────────────────

    /// All blocks (unordered).
    pub fn all(&self) -> Vec<Block> {
        self.blocks.iter().map(|r| r.value().clone()).collect()
    }

    /// Blocks in a given zone.
    pub fn by_zone(&self, zone: &Zone) -> Vec<Block> {
        self.blocks
            .iter()
            .filter(|r| &r.value().zone == zone)
            .map(|r| r.value().clone())
            .collect()
    }

    /// Blocks with a given role.
    pub fn by_role(&self, role: Role) -> Vec<Block> {
        self.blocks
            .iter()
            .filter(|r| r.value().role == role)
            .map(|r| r.value().clone())
            .collect()
    }

    /// Blocks sorted by timestamp (ascending).
    pub fn all_ordered(&self) -> Vec<Block> {
        let mut blocks = self.all();
        blocks.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        blocks
    }

    // ── Aggregates ────────────────────────────────────────────────

    /// Total number of blocks.
    pub fn count(&self) -> usize {
        self.blocks.len()
    }

    /// Sum of all block token counts.
    pub fn total_tokens(&self) -> u32 {
        self.blocks.iter().map(|r| r.value().tokens).sum()
    }

    /// Token totals grouped by zone.
    pub fn tokens_by_zone(&self) -> HashMap<String, u32> {
        let mut map: HashMap<String, u32> = HashMap::new();
        for entry in self.blocks.iter() {
            let key = zone_key(&entry.value().zone);
            *map.entry(key).or_default() += entry.value().tokens;
        }
        map
    }

    /// Token totals grouped by role.
    pub fn tokens_by_role(&self) -> HashMap<Role, u32> {
        let mut map: HashMap<Role, u32> = HashMap::new();
        for entry in self.blocks.iter() {
            *map.entry(entry.value().role).or_default() += entry.value().tokens;
        }
        map
    }

    // ── Lifecycle ─────────────────────────────────────────────────

    /// Remove all blocks.
    pub fn clear(&self) {
        self.blocks.clear();
    }

    /// Replace all blocks atomically (clear + insert).
    pub fn replace_all(&self, blocks: Vec<Block>) {
        self.blocks.clear();
        for block in blocks {
            self.blocks.insert(block.id.clone(), block);
        }
    }

    /// All block IDs.
    pub fn ids(&self) -> Vec<String> {
        self.blocks.iter().map(|r| r.key().clone()).collect()
    }
}

impl Default for BlockStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a Zone to a string key for aggregation maps.
fn zone_key(zone: &Zone) -> String {
    match zone {
        Zone::BuiltIn(b) => format!("{b:?}").to_lowercase(),
        Zone::Custom(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel};

    fn make_test_block(id: &str, role: Role, zone: Zone, tokens: u32) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: "test content".to_string(),
            tokens,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone,
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "test content".to_string(),
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

    #[test]
    fn test_insert_and_get() {
        let store = BlockStore::new();
        let block = make_test_block("b1", Role::User, Zone::BuiltIn(BuiltInZone::Middle), 100);
        store.insert(block);

        let retrieved = store.get("b1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().tokens, 100);
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let store = BlockStore::new();
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn test_update_modifies_block() {
        let store = BlockStore::new();
        let block = make_test_block("b1", Role::User, Zone::BuiltIn(BuiltInZone::Middle), 100);
        store.insert(block);

        let updated = store.update("b1", |b| b.tokens = 200);
        assert!(updated);
        assert_eq!(store.get("b1").unwrap().tokens, 200);
    }

    #[test]
    fn test_update_nonexistent_returns_false() {
        let store = BlockStore::new();
        assert!(!store.update("missing", |_| {}));
    }

    #[test]
    fn test_remove() {
        let store = BlockStore::new();
        let block = make_test_block("b1", Role::User, Zone::BuiltIn(BuiltInZone::Middle), 100);
        store.insert(block);

        let removed = store.remove("b1");
        assert!(removed.is_some());
        assert!(store.get("b1").is_none());
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_batch_operations() {
        let store = BlockStore::new();
        let blocks = vec![
            make_test_block("b1", Role::User, Zone::BuiltIn(BuiltInZone::Middle), 50),
            make_test_block(
                "b2",
                Role::Assistant,
                Zone::BuiltIn(BuiltInZone::Middle),
                100,
            ),
            make_test_block("b3", Role::System, Zone::BuiltIn(BuiltInZone::Primacy), 30),
        ];
        store.insert_many(blocks);
        assert_eq!(store.count(), 3);

        let got = store.get_many(&["b1".into(), "b3".into()]);
        assert_eq!(got.len(), 2);

        let removed = store.remove_many(&["b1".into(), "b2".into()]);
        assert_eq!(removed.len(), 2);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_by_zone() {
        let store = BlockStore::new();
        store.insert(make_test_block(
            "b1",
            Role::System,
            Zone::BuiltIn(BuiltInZone::Primacy),
            30,
        ));
        store.insert(make_test_block(
            "b2",
            Role::User,
            Zone::BuiltIn(BuiltInZone::Middle),
            50,
        ));
        store.insert(make_test_block(
            "b3",
            Role::Assistant,
            Zone::BuiltIn(BuiltInZone::Middle),
            100,
        ));

        let middle = store.by_zone(&Zone::BuiltIn(BuiltInZone::Middle));
        assert_eq!(middle.len(), 2);

        let primacy = store.by_zone(&Zone::BuiltIn(BuiltInZone::Primacy));
        assert_eq!(primacy.len(), 1);
    }

    #[test]
    fn test_by_role() {
        let store = BlockStore::new();
        store.insert(make_test_block(
            "b1",
            Role::User,
            Zone::BuiltIn(BuiltInZone::Middle),
            50,
        ));
        store.insert(make_test_block(
            "b2",
            Role::User,
            Zone::BuiltIn(BuiltInZone::Recency),
            60,
        ));
        store.insert(make_test_block(
            "b3",
            Role::Assistant,
            Zone::BuiltIn(BuiltInZone::Middle),
            100,
        ));

        assert_eq!(store.by_role(Role::User).len(), 2);
        assert_eq!(store.by_role(Role::Assistant).len(), 1);
        assert_eq!(store.by_role(Role::System).len(), 0);
    }

    #[test]
    fn test_aggregates() {
        let store = BlockStore::new();
        store.insert(make_test_block(
            "b1",
            Role::System,
            Zone::BuiltIn(BuiltInZone::Primacy),
            30,
        ));
        store.insert(make_test_block(
            "b2",
            Role::User,
            Zone::BuiltIn(BuiltInZone::Middle),
            50,
        ));
        store.insert(make_test_block(
            "b3",
            Role::Assistant,
            Zone::BuiltIn(BuiltInZone::Recency),
            120,
        ));

        assert_eq!(store.total_tokens(), 200);

        let by_zone = store.tokens_by_zone();
        assert_eq!(by_zone.get("primacy"), Some(&30));
        assert_eq!(by_zone.get("middle"), Some(&50));
        assert_eq!(by_zone.get("recency"), Some(&120));

        let by_role = store.tokens_by_role();
        assert_eq!(by_role.get(&Role::System), Some(&30));
    }

    #[test]
    fn test_replace_all() {
        let store = BlockStore::new();
        store.insert(make_test_block(
            "old",
            Role::User,
            Zone::BuiltIn(BuiltInZone::Middle),
            100,
        ));

        store.replace_all(vec![
            make_test_block("new1", Role::User, Zone::BuiltIn(BuiltInZone::Middle), 50),
            make_test_block("new2", Role::User, Zone::BuiltIn(BuiltInZone::Middle), 60),
        ]);

        assert!(store.get("old").is_none());
        assert_eq!(store.count(), 2);
        assert_eq!(store.total_tokens(), 110);
    }
}
