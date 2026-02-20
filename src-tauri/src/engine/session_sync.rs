use tracing::warn;

use super::*;
use crate::events::types::ApertureEvent;

impl ContextEngine {
    /// Persist the currently active session and its blocks.
    pub(crate) fn persist_active_session(&self) {
        if let Some(session_id) = self.sessions.active_id() {
            self.persist_session(&session_id);
        }
    }

    /// Keep active session block list/token totals aligned with the current in-memory store.
    pub(crate) fn refresh_active_session_totals(&self) {
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
    pub(crate) fn persist_session(&self, session_id: &str) {
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

    /// Block count and total token sum for the active session only.
    ///
    /// Falls back to global store counts when no session is active (e.g. during
    /// startup before the first ingest). Overhead tokens are included so the UI
    /// figure matches what `budget_status()` reports.
    pub(crate) fn active_session_counts(&self) -> (u32, u32) {
        match self.sessions.active() {
            Some(s) => {
                let tokens = s.total_tokens.saturating_add(s.overhead_tokens);
                (s.block_ids.len() as u32, tokens)
            }
            None => (self.store.count() as u32, self.store.total_tokens()),
        }
    }

    /// Emit a ContextUpdated event to the frontend.
    pub(crate) fn emit_context_updated(&self, block_count: u32, total_tokens: u32) {
        if let Some(ref dispatcher) = self.dispatcher {
            dispatcher.emit(&ApertureEvent::ContextUpdated {
                block_count,
                total_tokens,
            });
        }
    }
}
