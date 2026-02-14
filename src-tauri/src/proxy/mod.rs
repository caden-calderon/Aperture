//! HTTP proxy for intercepting LLM API requests.
//!
//! This module provides a transparent proxy that sits between AI tools
//! (Claude Code, Codex, etc.) and their upstream APIs. It captures
//! requests and responses for visualization while streaming SSE
//! responses back to clients.

pub mod capture;
pub mod context_api;
pub mod error;
pub mod handler;
pub mod hot_patch;
pub mod interceptor;
pub mod parser;
pub mod provider_adapter;
pub mod rewriter;

use axum::{routing::any, Router};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::info;

use self::capture::CaptureStore;
use self::error::ProxyError;
use self::hot_patch::HotPatchQueue;
use crate::engine::ContextEngine;
use crate::events::dispatcher::DynDispatcher;

/// Default port for the proxy server.
pub const DEFAULT_PORT: u16 = 5400;

/// Maximum request body size (10 MB).
pub(crate) const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Upstream API configuration.
#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    /// Base URL for Anthropic API.
    pub anthropic_url: String,
    /// Base URL for OpenAI API (for `sk-*` API keys).
    pub openai_url: String,
    /// Base URL for ChatGPT/Codex subscription backend.
    /// Used when a non-`sk-` Bearer token hits the Responses API path.
    pub chatgpt_codex_url: String,
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            anthropic_url: "https://api.anthropic.com".to_string(),
            openai_url: "https://api.openai.com".to_string(),
            chatgpt_codex_url: "https://chatgpt.com/backend-api/codex".to_string(),
        }
    }
}

/// Shared proxy state.
#[derive(Clone)]
pub struct ProxyState {
    pub(crate) client: Client,
    pub(crate) config: UpstreamConfig,
    /// Captured request/response exchanges.
    pub capture: CaptureStore,
    /// Event dispatcher for sending events to the Tauri frontend.
    /// `None` when running without a Tauri window (e.g., tests).
    pub(crate) dispatcher: Option<DynDispatcher>,
    /// Pending hot patches to apply on the next outbound request.
    pub hot_patches: Arc<HotPatchQueue>,
    /// Context engine for block processing, sessions, and persistence.
    /// `None` when running without the engine (e.g., basic tests).
    pub engine: Option<Arc<ContextEngine>>,
}

impl ProxyState {
    fn build_client(builder: reqwest::ClientBuilder) -> Result<Client, ProxyError> {
        builder.build().map_err(ProxyError::ClientBuildFailed)
    }

    /// Build the standard reqwest client for proxying.
    ///
    /// Auto-decompression is DISABLED — the proxy passes raw bytes through.
    /// We also strip `Accept-Encoding` from forwarded request headers so
    /// upstreams send uncompressed responses. This avoids Content-Length
    /// mismatches (where reqwest decompresses but we forward the compressed
    /// Content-Length) that cause "stream disconnected" errors.
    fn proxy_client_builder() -> reqwest::ClientBuilder {
        Client::builder()
            .no_gzip()
            .no_deflate()
            .no_brotli()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
    }

    /// Create new proxy state with default configuration (no event dispatch).
    pub fn new() -> Result<Self, ProxyError> {
        Self::with_config(UpstreamConfig::default())
    }

    /// Create proxy state with custom upstream configuration.
    pub fn with_config(config: UpstreamConfig) -> Result<Self, ProxyError> {
        let client = Self::build_client(Self::proxy_client_builder())?;
        Ok(Self {
            client,
            config,
            capture: CaptureStore::default(),
            dispatcher: None,
            hot_patches: Arc::new(HotPatchQueue::new()),
            engine: None,
        })
    }

    /// Attach an event dispatcher.
    pub fn with_dispatcher(mut self, dispatcher: DynDispatcher) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }
}

/// Start the proxy server.
pub async fn start_proxy(
    port: u16,
    dispatcher: Option<DynDispatcher>,
    hot_patches: Option<Arc<HotPatchQueue>>,
    engine: Option<Arc<ContextEngine>>,
) -> Result<(), ProxyError> {
    let mut state = ProxyState::new()?;
    if let Some(d) = dispatcher {
        state = state.with_dispatcher(d);
    }
    if let Some(hp) = hot_patches {
        state.hot_patches = hp;
    }
    state.engine = engine;
    let state = Arc::new(state);

    let app = Router::new()
        .route("/{*path}", any(handler::proxy_handler))
        .route("/", any(handler::proxy_handler))
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    info!("Starting proxy server on {}", addr);

    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| ProxyError::BindFailed {
            address: addr.clone(),
            source: e,
        })?;

    axum::serve(listener, app)
        .await
        .map_err(|e| ProxyError::BindFailed {
            address: addr,
            source: e,
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_state_with_config_uses_custom_urls() {
        let config = UpstreamConfig {
            anthropic_url: "https://example-anthropic.invalid".to_string(),
            openai_url: "https://example-openai.invalid".to_string(),
            chatgpt_codex_url: "https://example-chatgpt.invalid".to_string(),
        };

        let state = ProxyState::with_config(config.clone()).expect("should build client");
        assert_eq!(state.config.anthropic_url, config.anthropic_url);
        assert_eq!(state.config.openai_url, config.openai_url);
    }

    #[test]
    fn test_build_client_maps_reqwest_builder_errors() {
        let builder = Client::builder().user_agent("invalid\nuser-agent");
        let result = ProxyState::build_client(builder);

        assert!(matches!(result, Err(ProxyError::ClientBuildFailed(_))));
    }

    #[test]
    fn test_proxy_state_has_capture_store() {
        let state = ProxyState::new().unwrap();
        assert!(state.dispatcher.is_none());
        // CaptureStore is always present
        assert!(state.capture.get_exchange("nonexistent").is_none());
    }
}
