//! SQLite persistence layer.
//!
//! Background sync from the in-memory `BlockStore` to SQLite for
//! persistence and historical access. The in-memory store is always
//! the source of truth for the current session.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection, Result as SqlResult};
use tracing::{debug, info};

use super::block::Block;
use super::session::Session;
use super::versioning::BlockVersion;

/// SQLite storage backend.
pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    /// Open or create the database at the given path.
    pub fn open(path: &PathBuf) -> Result<Self, StorageError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::Io(e.to_string()))?;
        }

        let conn = Connection::open(path).map_err(StorageError::Sqlite)?;

        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.init_schema()?;

        info!("SQLite storage opened at {}", path.display());
        Ok(storage)
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory().map_err(StorageError::Sqlite)?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    /// Initialize database schema.
    fn init_schema(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                started_at TEXT NOT NULL,
                token_budget INTEGER NOT NULL,
                exchange_count INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS blocks (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tokens INTEGER NOT NULL,
                zone TEXT NOT NULL,
                compression_level TEXT NOT NULL DEFAULT 'original',
                timestamp TEXT NOT NULL,
                turn_index INTEGER NOT NULL DEFAULT 0,
                provider TEXT NOT NULL DEFAULT '',
                pinned TEXT,
                usage_heat REAL NOT NULL DEFAULT 0.0,
                reference_count INTEGER NOT NULL DEFAULT 0,
                metadata_json TEXT,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );

            CREATE TABLE IF NOT EXISTS block_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                content TEXT NOT NULL,
                tokens INTEGER NOT NULL,
                edited_at TEXT NOT NULL,
                source TEXT NOT NULL,
                FOREIGN KEY (block_id) REFERENCES blocks(id)
            );

            CREATE INDEX IF NOT EXISTS idx_blocks_session ON blocks(session_id);
            CREATE INDEX IF NOT EXISTS idx_blocks_zone ON blocks(zone);
            CREATE INDEX IF NOT EXISTS idx_block_versions_block ON block_versions(block_id);
            ",
        )
        .map_err(StorageError::Sqlite)?;

        debug!("SQLite schema initialized");
        Ok(())
    }

    // ── Session Operations ────────────────────────────────────

    /// Save a session to the database.
    pub fn save_session(&self, session: &Session) -> Result<(), StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        conn.execute(
            "INSERT OR REPLACE INTO sessions (id, provider, model, started_at, token_budget, exchange_count, total_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.id,
                session.provider,
                session.model,
                session.started_at,
                session.token_budget,
                session.exchange_count,
                session.total_tokens,
            ],
        )
        .map_err(StorageError::Sqlite)?;

        Ok(())
    }

    /// Load all sessions.
    pub fn load_sessions(&self) -> Result<Vec<Session>, StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        let mut stmt = conn
            .prepare(
                "SELECT id, provider, model, started_at, token_budget, exchange_count, total_tokens FROM sessions",
            )
            .map_err(StorageError::Sqlite)?;

        let sessions = stmt
            .query_map([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    model: row.get(2)?,
                    source: "unknown".to_string(),
                    thread_identity: None,
                    started_at: row.get(3)?,
                    token_budget: row.get(4)?,
                    exchange_count: row.get(5)?,
                    total_tokens: row.get(6)?,
                    block_ids: Vec::new(), // populated from blocks table
                })
            })
            .map_err(StorageError::Sqlite)?
            .collect::<SqlResult<Vec<_>>>()
            .map_err(StorageError::Sqlite)?;

        Ok(sessions)
    }

    // ── Block Operations ──────────────────────────────────────

    /// Save a block to the database.
    pub fn save_block(&self, block: &Block, session_id: Option<&str>) -> Result<(), StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        let zone_str = zone_to_string(&block.zone);
        let pinned_str = block
            .pinned
            .as_ref()
            .map(|p| format!("{p:?}").to_lowercase());
        let metadata_json = serde_json::to_string(&block.metadata).unwrap_or_else(|_| "{}".into());

        conn.execute(
            "INSERT OR REPLACE INTO blocks (id, session_id, role, content, tokens, zone, compression_level, timestamp, turn_index, provider, pinned, usage_heat, reference_count, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                block.id,
                session_id,
                format!("{:?}", block.role).to_lowercase(),
                block.content,
                block.tokens,
                zone_str,
                format!("{:?}", block.compression_level).to_lowercase(),
                block.timestamp,
                block.metadata.turn_index,
                block.metadata.provider,
                pinned_str,
                block.usage_heat,
                block.reference_count,
                metadata_json,
            ],
        )
        .map_err(StorageError::Sqlite)?;

        Ok(())
    }

    /// Save many blocks in a single transaction.
    pub fn save_blocks(
        &self,
        blocks: &[Block],
        session_id: Option<&str>,
    ) -> Result<(), StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        let tx = conn.unchecked_transaction().map_err(StorageError::Sqlite)?;

        // Session snapshots are replaced wholesale on each ingest, so remove
        // old persisted rows for that session before writing the new set.
        if let Some(session_id) = session_id {
            tx.execute(
                "DELETE FROM block_versions WHERE block_id IN (SELECT id FROM blocks WHERE session_id = ?1)",
                params![session_id],
            )
            .map_err(StorageError::Sqlite)?;
            tx.execute(
                "DELETE FROM blocks WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(StorageError::Sqlite)?;
        }

        for block in blocks {
            let zone_str = zone_to_string(&block.zone);
            let pinned_str = block
                .pinned
                .as_ref()
                .map(|p| format!("{p:?}").to_lowercase());
            let metadata_json =
                serde_json::to_string(&block.metadata).unwrap_or_else(|_| "{}".into());

            tx.execute(
                "INSERT OR REPLACE INTO blocks (id, session_id, role, content, tokens, zone, compression_level, timestamp, turn_index, provider, pinned, usage_heat, reference_count, metadata_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    block.id,
                    session_id,
                    format!("{:?}", block.role).to_lowercase(),
                    block.content,
                    block.tokens,
                    zone_str,
                    format!("{:?}", block.compression_level).to_lowercase(),
                    block.timestamp,
                    block.metadata.turn_index,
                    block.metadata.provider,
                    pinned_str,
                    block.usage_heat,
                    block.reference_count,
                    metadata_json,
                ],
            )
            .map_err(StorageError::Sqlite)?;
        }

        tx.commit().map_err(StorageError::Sqlite)?;
        Ok(())
    }

    /// Load block IDs for a session.
    pub fn load_block_ids(&self, session_id: &str) -> Result<Vec<String>, StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        let mut stmt = conn
            .prepare("SELECT id FROM blocks WHERE session_id = ?1 ORDER BY turn_index")
            .map_err(StorageError::Sqlite)?;

        let ids = stmt
            .query_map(params![session_id], |row| row.get(0))
            .map_err(StorageError::Sqlite)?
            .collect::<SqlResult<Vec<String>>>()
            .map_err(StorageError::Sqlite)?;

        Ok(ids)
    }

    // ── Version Operations ────────────────────────────────────

    /// Save a block version.
    pub fn save_version(&self, block_id: &str, version: &BlockVersion) -> Result<(), StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        conn.execute(
            "INSERT INTO block_versions (block_id, version, content, tokens, edited_at, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                block_id,
                version.version,
                version.content,
                version.tokens,
                version.edited_at,
                format!("{:?}", version.source).to_lowercase(),
            ],
        )
        .map_err(StorageError::Sqlite)?;

        Ok(())
    }

    /// Load version history for a block.
    pub fn load_versions(&self, block_id: &str) -> Result<Vec<BlockVersion>, StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        let mut stmt = conn
            .prepare(
                "SELECT version, content, tokens, edited_at, source
                 FROM block_versions WHERE block_id = ?1 ORDER BY version DESC",
            )
            .map_err(StorageError::Sqlite)?;

        let versions = stmt
            .query_map(params![block_id], |row| {
                let source_str: String = row.get(4)?;
                Ok(BlockVersion {
                    version: row.get(0)?,
                    content: row.get(1)?,
                    tokens: row.get(2)?,
                    edited_at: row.get(3)?,
                    source: parse_edit_source(&source_str),
                })
            })
            .map_err(StorageError::Sqlite)?
            .collect::<SqlResult<Vec<_>>>()
            .map_err(StorageError::Sqlite)?;

        Ok(versions)
    }

    // ── Cleanup ───────────────────────────────────────────────

    /// Delete all data for a session.
    pub fn delete_session(&self, session_id: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;

        // Delete versions for session's blocks
        conn.execute(
            "DELETE FROM block_versions WHERE block_id IN (SELECT id FROM blocks WHERE session_id = ?1)",
            params![session_id],
        )
        .map_err(StorageError::Sqlite)?;

        // Delete blocks
        conn.execute(
            "DELETE FROM blocks WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(StorageError::Sqlite)?;

        // Delete session
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
            .map_err(StorageError::Sqlite)?;

        Ok(())
    }
}

/// Storage error types.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Lock poisoned")]
    LockPoisoned,
}

/// Get the default database path.
pub fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".aperture").join("aperture.db")
}

fn zone_to_string(zone: &super::types::Zone) -> String {
    use super::types::{BuiltInZone, Zone};
    match zone {
        Zone::BuiltIn(BuiltInZone::Primacy) => "primacy".into(),
        Zone::BuiltIn(BuiltInZone::Middle) => "middle".into(),
        Zone::BuiltIn(BuiltInZone::Recency) => "recency".into(),
        Zone::Custom(s) => s.clone(),
    }
}

fn parse_edit_source(s: &str) -> super::versioning::EditSource {
    use super::versioning::EditSource;
    match s {
        "user" => EditSource::User,
        "auto_rule" | "autorule" => EditSource::AutoRule,
        "hot_patch" | "hotpatch" => EditSource::HotPatch,
        "compression" => EditSource::Compression,
        "pipeline" => EditSource::Pipeline,
        _ => EditSource::User,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};
    use crate::engine::versioning::EditSource;

    fn make_test_block(id: &str) -> Block {
        Block {
            id: id.to_string(),
            role: Role::User,
            block_type: None,
            content: "test content".to_string(),
            tokens: 100,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "test content".to_string(),
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
    fn test_schema_creation() {
        let storage = SqliteStorage::open_memory().unwrap();
        // Schema created successfully if we get here
        assert!(storage.load_sessions().unwrap().is_empty());
    }

    #[test]
    fn test_save_and_load_session() {
        let storage = SqliteStorage::open_memory().unwrap();

        let session = Session::new(
            "s1".into(),
            "anthropic".into(),
            "claude-opus-4-6".into(),
            "proxy".into(),
            None,
            200_000,
        );
        storage.save_session(&session).unwrap();

        let sessions = storage.load_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(sessions[0].provider, "anthropic");
    }

    #[test]
    fn test_save_and_load_blocks() {
        let storage = SqliteStorage::open_memory().unwrap();

        let session = Session::new(
            "s1".into(),
            "test".into(),
            "model".into(),
            "proxy".into(),
            None,
            100_000,
        );
        storage.save_session(&session).unwrap();

        let block = make_test_block("b1");
        storage.save_block(&block, Some("s1")).unwrap();

        let ids = storage.load_block_ids("s1").unwrap();
        assert_eq!(ids, vec!["b1"]);
    }

    #[test]
    fn test_save_blocks_batch() {
        let storage = SqliteStorage::open_memory().unwrap();

        let session = Session::new(
            "s1".into(),
            "test".into(),
            "model".into(),
            "proxy".into(),
            None,
            100_000,
        );
        storage.save_session(&session).unwrap();

        let blocks = vec![
            make_test_block("b1"),
            make_test_block("b2"),
            make_test_block("b3"),
        ];
        storage.save_blocks(&blocks, Some("s1")).unwrap();

        let ids = storage.load_block_ids("s1").unwrap();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn test_save_blocks_replaces_existing_session_rows() {
        let storage = SqliteStorage::open_memory().unwrap();

        let session = Session::new(
            "s1".into(),
            "test".into(),
            "model".into(),
            "proxy".into(),
            None,
            100_000,
        );
        storage.save_session(&session).unwrap();

        storage
            .save_blocks(
                &[make_test_block("old1"), make_test_block("old2")],
                Some("s1"),
            )
            .unwrap();
        let before = storage.load_block_ids("s1").unwrap();
        assert_eq!(before.len(), 2);

        storage
            .save_blocks(&[make_test_block("new1")], Some("s1"))
            .unwrap();
        let after = storage.load_block_ids("s1").unwrap();
        assert_eq!(after, vec!["new1".to_string()]);
    }

    #[test]
    fn test_save_and_load_versions() {
        let storage = SqliteStorage::open_memory().unwrap();

        // Insert a block first to satisfy FK constraint
        let block = make_test_block("b1");
        storage.save_block(&block, None).unwrap();

        let version = BlockVersion {
            version: 1,
            content: "original content".into(),
            tokens: 50,
            edited_at: "2026-01-01T00:00:00Z".into(),
            source: EditSource::User,
        };
        storage.save_version("b1", &version).unwrap();

        let versions = storage.load_versions("b1").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].content, "original content");
    }

    #[test]
    fn test_delete_session_cascades() {
        let storage = SqliteStorage::open_memory().unwrap();

        let session = Session::new(
            "s1".into(),
            "test".into(),
            "model".into(),
            "proxy".into(),
            None,
            100_000,
        );
        storage.save_session(&session).unwrap();

        let block = make_test_block("b1");
        storage.save_block(&block, Some("s1")).unwrap();

        storage.delete_session("s1").unwrap();

        assert!(storage.load_sessions().unwrap().is_empty());
        assert!(storage.load_block_ids("s1").unwrap().is_empty());
    }
}
