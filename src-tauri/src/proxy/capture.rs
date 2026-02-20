//! Request/response capture for context extraction.
//!
//! Captures API traffic flowing through the proxy, parses it into blocks,
//! and stores it for event emission and UI consumption. Handles both
//! synchronous responses and SSE streaming accumulation.

mod sse;
#[cfg(test)]
mod tests;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, warn};

use self::sse::extract_final_response;
use super::parser::{self, ParsedRequest, Provider, TokenUsage};
use crate::engine::block::Block;

/// State for a single in-flight request.
#[derive(Debug, Clone)]
pub struct CapturedExchange {
    pub request_id: String,
    pub provider: Provider,
    pub model: String,
    pub thread_identity: Option<String>,
    pub path: String,
    pub request_blocks: Vec<Block>,
    pub response_blocks: Vec<Block>,
    pub usage: Option<TokenUsage>,
    pub status: ExchangeStatus,
    /// Accumulated SSE chunks for streaming responses.
    pub sse_buffer: String,
    /// Total bytes received during streaming.
    pub bytes_received: u64,
    /// Estimated token overhead from tool definitions (bytes/4 of tools array).
    pub overhead_tokens: u32,
}

/// Status of an exchange in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeStatus {
    /// Request captured, waiting for response.
    Pending,
    /// Response streaming in progress.
    Streaming,
    /// Response fully received and parsed.
    Complete,
    /// An error occurred.
    Failed,
}

/// Manages captured request/response exchanges.
///
/// Thread-safe via `DashMap` - proxy handlers run concurrently.
#[derive(Debug, Clone)]
pub struct CaptureStore {
    /// In-flight and recently completed exchanges, keyed by request_id.
    exchanges: Arc<DashMap<String, CapturedExchange>>,
    /// Maximum number of completed exchanges to retain.
    max_retained: usize,
}

impl Default for CaptureStore {
    fn default() -> Self {
        Self::new(100)
    }
}

impl CaptureStore {
    pub fn new(max_retained: usize) -> Self {
        Self {
            exchanges: Arc::new(DashMap::new()),
            max_retained,
        }
    }

    /// Capture and parse an incoming request.
    ///
    /// Returns the parsed request data (provider, blocks) for the handler to use.
    pub fn capture_request(
        &self,
        request_id: &str,
        path: &str,
        body: &[u8],
    ) -> Option<ParsedRequest> {
        // Only parse API endpoints we understand
        if !is_api_endpoint(path) {
            return None;
        }

        match parser::parse_request(path, body) {
            Ok(parsed) => {
                let exchange = CapturedExchange {
                    request_id: request_id.to_string(),
                    provider: parsed.provider,
                    model: parsed.model.clone(),
                    thread_identity: parsed.thread_identity.clone(),
                    path: path.to_string(),
                    request_blocks: parsed.blocks.clone(),
                    response_blocks: Vec::new(),
                    usage: None,
                    status: ExchangeStatus::Pending,
                    sse_buffer: String::new(),
                    bytes_received: 0,
                    overhead_tokens: parsed.overhead_tokens,
                };
                self.exchanges.insert(request_id.to_string(), exchange);
                self.evict_if_needed();
                debug!(
                    request_id,
                    provider = %parsed.provider,
                    blocks = parsed.blocks.len(),
                    "Captured request"
                );
                Some(parsed)
            }
            Err(e) => {
                warn!(request_id, error = %e, "Failed to parse request");
                None
            }
        }
    }

    /// Record an SSE chunk for a streaming response.
    ///
    /// Returns the total bytes received so far.
    pub fn append_sse_chunk(&self, request_id: &str, chunk: &[u8]) -> u64 {
        if let Some(mut exchange) = self.exchanges.get_mut(request_id) {
            exchange.status = ExchangeStatus::Streaming;
            exchange.bytes_received += chunk.len() as u64;

            // Accumulate text for later parsing
            if let Ok(text) = std::str::from_utf8(chunk) {
                exchange.sse_buffer.push_str(text);
            }

            exchange.bytes_received
        } else {
            0
        }
    }

    /// Finalize a streaming response by parsing the accumulated SSE buffer.
    ///
    /// Extracts the final response JSON from the SSE event stream and parses it.
    pub fn finalize_streaming(&self, request_id: &str) -> Option<CapturedExchange> {
        if let Some(mut exchange) = self.exchanges.get_mut(request_id) {
            // Extract the last complete data event from the SSE stream.
            // SSE format: "data: {json}\n\n" or "event: message_stop\ndata: ...\n\n"
            let response_json =
                extract_final_response(&exchange.sse_buffer, exchange.provider, &exchange.path);

            if let Some(json_bytes) = response_json {
                match parser::parse_response(exchange.provider, &exchange.path, &json_bytes) {
                    Ok(parsed) => {
                        exchange.response_blocks = parsed.blocks;
                        exchange.usage = parsed.usage;
                        exchange.status = ExchangeStatus::Complete;
                    }
                    Err(e) => {
                        warn!(request_id, error = %e, "Failed to parse streaming response");
                        exchange.status = ExchangeStatus::Complete;
                    }
                }
            } else {
                // No parseable response found in stream, still mark complete.
                exchange.status = ExchangeStatus::Complete;
            }

            Some(exchange.clone())
        } else {
            None
        }
    }

    /// Capture a complete (non-streaming) response.
    pub fn capture_response(
        &self,
        request_id: &str,
        status_code: u16,
        body: &[u8],
    ) -> Option<CapturedExchange> {
        if let Some(mut exchange) = self.exchanges.get_mut(request_id) {
            if status_code >= 400 {
                exchange.status = ExchangeStatus::Failed;
                return Some(exchange.clone());
            }

            match parser::parse_response(exchange.provider, &exchange.path, body) {
                Ok(parsed) => {
                    exchange.response_blocks = parsed.blocks;
                    exchange.usage = parsed.usage;
                    exchange.status = ExchangeStatus::Complete;
                }
                Err(e) => {
                    warn!(request_id, error = %e, "Failed to parse response");
                    exchange.status = ExchangeStatus::Failed;
                }
            }

            Some(exchange.clone())
        } else {
            None
        }
    }

    /// Mark an exchange as failed.
    pub fn mark_failed(&self, request_id: &str) {
        if let Some(mut exchange) = self.exchanges.get_mut(request_id) {
            exchange.status = ExchangeStatus::Failed;
        }
    }

    /// Get a snapshot of a captured exchange.
    pub fn get_exchange(&self, request_id: &str) -> Option<CapturedExchange> {
        self.exchanges.get(request_id).map(|e| e.clone())
    }

    /// Get all blocks from all completed exchanges (request + response).
    pub fn all_blocks(&self) -> Vec<Block> {
        let mut blocks = Vec::new();
        for entry in self.exchanges.iter() {
            let exchange = entry.value();
            if exchange.status == ExchangeStatus::Complete {
                blocks.extend(exchange.request_blocks.iter().cloned());
                blocks.extend(exchange.response_blocks.iter().cloned());
            }
        }
        blocks
    }

    /// Evict oldest completed exchanges if we exceed the limit.
    fn evict_if_needed(&self) {
        let completed_count = self
            .exchanges
            .iter()
            .filter(|e| e.value().status == ExchangeStatus::Complete)
            .count();

        if completed_count > self.max_retained {
            let to_remove = completed_count - self.max_retained;
            let keys_to_remove: Vec<String> = self
                .exchanges
                .iter()
                .filter(|e| e.value().status == ExchangeStatus::Complete)
                .take(to_remove)
                .map(|e| e.key().clone())
                .collect();
            for key in &keys_to_remove {
                self.exchanges.remove(key);
            }
        }
    }
}

/// Check if a path is an API endpoint we should capture.
pub(super) fn is_api_endpoint(path: &str) -> bool {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};
    is_messages_path(path) || is_chat_completions_path(path) || is_responses_path(path)
}
