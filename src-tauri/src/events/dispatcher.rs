//! Event dispatcher for broadcasting events to the Tauri frontend.
//!
//! Wraps the Tauri `AppHandle` and provides typed event emission
//! using the `ApertureEvent` enum and dedicated channel names.

use tauri::{AppHandle, Emitter, Runtime};
use tracing::{debug, warn};

use super::types::{channels, ApertureEvent};
use crate::proxy::capture::CapturedExchange;

/// Broadcasts `ApertureEvent` variants to the Tauri frontend.
///
/// Clone-cheap: holds an `AppHandle` which is internally `Arc`-backed.
#[derive(Clone)]
pub struct EventDispatcher<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> EventDispatcher<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }

    /// Emit an event on the main aperture events channel.
    pub fn emit(&self, event: &ApertureEvent) {
        let channel = match event {
            ApertureEvent::ResponseStreaming { .. } => channels::STREAM_PROGRESS,
            _ => channels::APERTURE_EVENTS,
        };

        if let Err(e) = self.app.emit(channel, event) {
            warn!(channel, error = %e, "Failed to emit event");
        } else {
            debug!(channel, ?event, "Emitted event");
        }
    }

    /// Emit a request captured event.
    pub fn request_captured(&self, request_id: &str, method: &str, path: &str, provider: &str) {
        self.emit(&ApertureEvent::RequestCaptured {
            request_id: request_id.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            provider: provider.to_string(),
        });
    }

    /// Emit a streaming progress event.
    pub fn response_streaming(&self, request_id: &str, bytes_received: u64) {
        self.emit(&ApertureEvent::ResponseStreaming {
            request_id: request_id.to_string(),
            bytes_received,
        });
    }

    /// Emit a response complete event.
    pub fn response_complete(&self, request_id: &str, status: u16, tokens_used: Option<u32>) {
        self.emit(&ApertureEvent::ResponseComplete {
            request_id: request_id.to_string(),
            status,
            tokens_used,
        });
    }

    /// Emit a context updated event.
    pub fn context_updated(&self, block_count: u32, total_tokens: u32) {
        self.emit(&ApertureEvent::ContextUpdated {
            block_count,
            total_tokens,
        });
    }

    /// Emit a blocks captured event from a completed exchange.
    pub fn blocks_captured(&self, exchange: &CapturedExchange) {
        self.emit(&ApertureEvent::BlocksCaptured {
            request_id: exchange.request_id.clone(),
            provider: exchange.provider.to_string(),
            model: exchange.model.clone(),
            request_blocks: exchange.request_blocks.clone(),
            response_blocks: exchange.response_blocks.clone(),
            input_tokens: exchange.usage.as_ref().map(|u| u.input_tokens),
            output_tokens: exchange.usage.as_ref().map(|u| u.output_tokens),
        });
    }

    /// Emit a proxy error event.
    pub fn proxy_error(&self, request_id: Option<&str>, message: &str) {
        self.emit(&ApertureEvent::ProxyError {
            request_id: request_id.map(|s| s.to_string()),
            message: message.to_string(),
        });
    }
}

/// A type-erased dispatcher for use in non-generic contexts (like axum handlers).
///
/// The proxy handler can't be generic over `R: Runtime` because axum needs
/// concrete types. This wrapper uses dynamic dispatch via `dyn Fn`.
#[derive(Clone)]
pub struct DynDispatcher {
    emit_fn: std::sync::Arc<dyn Fn(&ApertureEvent) + Send + Sync>,
}

impl DynDispatcher {
    /// Create from a concrete EventDispatcher.
    pub fn from_typed<R: Runtime>(dispatcher: EventDispatcher<R>) -> Self {
        Self {
            emit_fn: std::sync::Arc::new(move |event| dispatcher.emit(event)),
        }
    }

    /// Emit an event.
    pub fn emit(&self, event: &ApertureEvent) {
        (self.emit_fn)(event);
    }

    /// Convenience: emit request captured.
    pub fn request_captured(&self, request_id: &str, method: &str, path: &str, provider: &str) {
        self.emit(&ApertureEvent::RequestCaptured {
            request_id: request_id.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            provider: provider.to_string(),
        });
    }

    /// Convenience: emit streaming progress.
    pub fn response_streaming(&self, request_id: &str, bytes_received: u64) {
        self.emit(&ApertureEvent::ResponseStreaming {
            request_id: request_id.to_string(),
            bytes_received,
        });
    }

    /// Convenience: emit response complete.
    pub fn response_complete(&self, request_id: &str, status: u16, tokens_used: Option<u32>) {
        self.emit(&ApertureEvent::ResponseComplete {
            request_id: request_id.to_string(),
            status,
            tokens_used,
        });
    }

    /// Convenience: emit context updated.
    pub fn context_updated(&self, block_count: u32, total_tokens: u32) {
        self.emit(&ApertureEvent::ContextUpdated {
            block_count,
            total_tokens,
        });
    }

    /// Convenience: emit blocks captured from a completed exchange.
    pub fn blocks_captured(&self, exchange: &CapturedExchange) {
        self.emit(&ApertureEvent::BlocksCaptured {
            request_id: exchange.request_id.clone(),
            provider: exchange.provider.to_string(),
            model: exchange.model.clone(),
            request_blocks: exchange.request_blocks.clone(),
            response_blocks: exchange.response_blocks.clone(),
            input_tokens: exchange.usage.as_ref().map(|u| u.input_tokens),
            output_tokens: exchange.usage.as_ref().map(|u| u.output_tokens),
        });
    }

    /// Convenience: emit proxy error.
    pub fn proxy_error(&self, request_id: Option<&str>, message: &str) {
        self.emit(&ApertureEvent::ProxyError {
            request_id: request_id.map(|s| s.to_string()),
            message: message.to_string(),
        });
    }
}

impl std::fmt::Debug for DynDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynDispatcher").finish()
    }
}
