//! Event type definitions.

use serde::{Deserialize, Serialize};

use crate::engine::block::Block;

/// Events emitted by the Aperture backend.
///
/// These are sent to the frontend via Tauri's event system
/// (`app_handle.emit()`). The frontend listens with `listen()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApertureEvent {
    /// A new API request was captured by the proxy.
    RequestCaptured {
        request_id: String,
        method: String,
        path: String,
        provider: String,
    },

    /// Parsed blocks from a captured request/response exchange.
    BlocksCaptured {
        request_id: String,
        provider: String,
        model: String,
        request_blocks: Vec<Block>,
        response_blocks: Vec<Block>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },

    /// An SSE response is being streamed.
    ResponseStreaming {
        request_id: String,
        /// Number of bytes received so far.
        bytes_received: u64,
    },

    /// A response has been fully received and processed.
    ResponseComplete {
        request_id: String,
        status: u16,
        tokens_used: Option<u32>,
    },

    /// The context model has been updated (blocks added/modified/removed).
    ContextUpdated { block_count: u32, total_tokens: u32 },

    /// An error occurred in the proxy.
    ProxyError {
        request_id: Option<String>,
        message: String,
    },
}

/// Event channel names used with Tauri's event system.
pub mod channels {
    /// Main event channel for all Aperture events.
    pub const APERTURE_EVENTS: &str = "aperture:events";

    /// Dedicated channel for high-frequency streaming updates.
    pub const STREAM_PROGRESS: &str = "aperture:stream-progress";
}
