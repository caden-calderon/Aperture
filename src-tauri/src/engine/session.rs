//! Session management for concurrent tool connections.
//!
//! Each session represents one tool connection (e.g., a Claude Code instance,
//! a Codex session). Sessions own a set of block IDs and track metadata.

use std::sync::Mutex;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

/// A single tool session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub source: String,
    #[serde(default)]
    pub thread_identity: Option<String>,
    pub started_at: String,
    pub token_budget: u32,
    /// Ordered list of block IDs belonging to this session.
    pub block_ids: Vec<String>,
    /// Number of request/response exchanges in this session.
    pub exchange_count: u32,
    /// Total tokens used across all blocks in this session.
    pub total_tokens: u32,
    /// Estimated token overhead from tool definitions and other non-message
    /// content that the LLM counts toward context. Updated each ingest.
    #[serde(default)]
    pub overhead_tokens: u32,
}

/// Summary info for session listing (without block IDs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub source: String,
    #[serde(default)]
    pub thread_identity: Option<String>,
    pub started_at: String,
    pub block_count: usize,
    pub exchange_count: u32,
    pub total_tokens: u32,
    pub token_budget: u32,
    /// Estimated token overhead from tool definitions (not tracked as blocks).
    #[serde(default)]
    pub overhead_tokens: u32,
}

impl Session {
    pub fn new(
        id: String,
        provider: String,
        model: String,
        source: String,
        thread_identity: Option<String>,
        token_budget: u32,
    ) -> Self {
        Self {
            id,
            provider,
            model,
            source,
            thread_identity,
            started_at: chrono_now(),
            token_budget,
            block_ids: Vec::new(),
            exchange_count: 0,
            total_tokens: 0,
            overhead_tokens: 0,
        }
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            source: self.source.clone(),
            thread_identity: self.thread_identity.clone(),
            started_at: self.started_at.clone(),
            block_count: self.block_ids.len(),
            exchange_count: self.exchange_count,
            total_tokens: self.total_tokens,
            token_budget: self.token_budget,
            overhead_tokens: self.overhead_tokens,
        }
    }

    /// Add block IDs from a new exchange.
    pub fn add_blocks(&mut self, block_ids: &[String], tokens: u32) {
        self.block_ids.extend_from_slice(block_ids);
        self.total_tokens += tokens;
        self.exchange_count += 1;
    }
}

/// Concurrent session store.
pub struct SessionStore {
    sessions: DashMap<String, Session>,
    active_id: Mutex<Option<String>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            active_id: Mutex::new(None),
        }
    }

    /// Create a new session and set it as active.
    ///
    /// Guard: if an active session exists with significant content (>1000 tokens),
    /// a new session with a much smaller budget indication won't steal active status.
    /// This prevents auxiliary model calls (e.g. Haiku topic classifier) from
    /// flipping the active session away from the main conversation.
    pub fn create(
        &self,
        id: String,
        provider: String,
        model: String,
        source: String,
        thread_identity: Option<String>,
        token_budget: u32,
    ) -> String {
        let session = Session::new(
            id.clone(),
            provider.clone(),
            model.clone(),
            source,
            thread_identity,
            token_budget,
        );
        self.sessions.insert(id.clone(), session);

        // Only flip active session if there isn't already a substantial one.
        // This prevents tiny auxiliary sessions (topic classifiers, ~120 tokens)
        // from stealing active status from the main conversation.
        let should_activate = {
            let current_active = self.active();
            match current_active {
                None => true,
                Some(active) => {
                    // If active session is small or same provider+model, allow flip
                    let active_is_small = active.total_tokens < 1000;
                    let same_model =
                        active.provider == provider && active.model == model;
                    active_is_small || same_model
                }
            }
        };

        if should_activate {
            *self.active_id.lock().expect("active_id lock poisoned") = Some(id.clone());
        }
        id
    }

    /// Get a session by ID.
    pub fn get(&self, id: &str) -> Option<Session> {
        self.sessions.get(id).map(|r| r.value().clone())
    }

    /// Get the currently active session.
    pub fn active(&self) -> Option<Session> {
        let active_id = self.active_id.lock().expect("active_id lock poisoned");
        active_id
            .as_ref()
            .and_then(|id| self.sessions.get(id).map(|r| r.value().clone()))
    }

    /// Get the active session ID.
    pub fn active_id(&self) -> Option<String> {
        self.active_id
            .lock()
            .expect("active_id lock poisoned")
            .clone()
    }

    /// Switch to a different session.
    pub fn switch_to(&self, id: &str) -> bool {
        if self.sessions.contains_key(id) {
            *self.active_id.lock().expect("active_id lock poisoned") = Some(id.to_string());
            true
        } else {
            false
        }
    }

    /// Update a session in-place.
    pub fn update<F: FnOnce(&mut Session)>(&self, id: &str, f: F) -> bool {
        if let Some(mut entry) = self.sessions.get_mut(id) {
            f(entry.value_mut());
            true
        } else {
            false
        }
    }

    /// List all sessions as summaries.
    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions.iter().map(|r| r.value().info()).collect()
    }

    /// Remove a session.
    pub fn remove(&self, id: &str) -> Option<Session> {
        let removed = self.sessions.remove(id).map(|(_, s)| s);
        // If we removed the active session, clear active_id
        if let Some(ref session) = removed {
            let mut active = self.active_id.lock().expect("active_id lock poisoned");
            if active.as_deref() == Some(&session.id) {
                *active = None;
            }
        }
        removed
    }

    /// Total session count.
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// Clear all sessions.
    pub fn clear(&self) {
        self.sessions.clear();
        *self.active_id.lock().expect("active_id lock poisoned") = None;
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

fn chrono_now() -> String {
    crate::util::iso_now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_session() {
        let store = SessionStore::new();
        let id = store.create(
            "s1".into(),
            "anthropic".into(),
            "claude-opus-4-6".into(),
            "proxy".into(),
            None,
            200_000,
        );

        assert_eq!(id, "s1");
        assert_eq!(store.count(), 1);

        let session = store.get("s1").unwrap();
        assert_eq!(session.provider, "anthropic");
        assert_eq!(session.source, "proxy");
        assert_eq!(session.thread_identity, None);
        assert_eq!(session.token_budget, 200_000);
        assert!(session.block_ids.is_empty());
    }

    #[test]
    fn test_active_session() {
        let store = SessionStore::new();
        store.create(
            "s1".into(),
            "anthropic".into(),
            "claude".into(),
            "proxy".into(),
            None,
            200_000,
        );
        store.create(
            "s2".into(),
            "openai".into(),
            "gpt-4o".into(),
            "direct_cli_bridge".into(),
            Some("thread-1".into()),
            128_000,
        );

        // Last created is active
        let active = store.active().unwrap();
        assert_eq!(active.id, "s2");
    }

    #[test]
    fn test_switch_session() {
        let store = SessionStore::new();
        store.create(
            "s1".into(),
            "anthropic".into(),
            "claude".into(),
            "proxy".into(),
            None,
            200_000,
        );
        store.create(
            "s2".into(),
            "openai".into(),
            "gpt-4o".into(),
            "direct_cli_bridge".into(),
            Some("thread-1".into()),
            128_000,
        );

        assert!(store.switch_to("s1"));
        assert_eq!(store.active_id(), Some("s1".into()));

        assert!(!store.switch_to("nonexistent"));
        assert_eq!(store.active_id(), Some("s1".into()));
    }

    #[test]
    fn test_add_blocks_to_session() {
        let store = SessionStore::new();
        store.create(
            "s1".into(),
            "anthropic".into(),
            "claude".into(),
            "proxy".into(),
            None,
            200_000,
        );

        store.update("s1", |s| {
            s.add_blocks(&["b1".into(), "b2".into()], 150);
        });

        let session = store.get("s1").unwrap();
        assert_eq!(session.block_ids.len(), 2);
        assert_eq!(session.total_tokens, 150);
        assert_eq!(session.exchange_count, 1);
    }

    #[test]
    fn test_session_list() {
        let store = SessionStore::new();
        store.create(
            "s1".into(),
            "anthropic".into(),
            "claude".into(),
            "proxy".into(),
            None,
            200_000,
        );
        store.create(
            "s2".into(),
            "openai".into(),
            "gpt-4o".into(),
            "proxy".into(),
            None,
            128_000,
        );

        let list = store.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_remove_active_session() {
        let store = SessionStore::new();
        store.create(
            "s1".into(),
            "anthropic".into(),
            "claude".into(),
            "proxy".into(),
            None,
            200_000,
        );

        store.remove("s1");
        assert!(store.active_id().is_none());
        assert_eq!(store.count(), 0);
    }
}
