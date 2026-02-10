//! Request/response capture for context extraction.
//!
//! Captures API traffic flowing through the proxy, parses it into blocks,
//! and stores it for event emission and UI consumption. Handles both
//! synchronous responses and SSE streaming accumulation.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, warn};

use super::parser::{self, ParsedRequest, Provider, TokenUsage};
use crate::engine::block::Block;

/// State for a single in-flight request.
#[derive(Debug, Clone)]
pub struct CapturedExchange {
    pub request_id: String,
    pub provider: Provider,
    pub model: String,
    pub path: String,
    pub request_blocks: Vec<Block>,
    pub response_blocks: Vec<Block>,
    pub usage: Option<TokenUsage>,
    pub status: ExchangeStatus,
    /// Accumulated SSE chunks for streaming responses.
    pub sse_buffer: String,
    /// Total bytes received during streaming.
    pub bytes_received: u64,
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
/// Thread-safe via `DashMap` — proxy handlers run concurrently.
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
                    path: path.to_string(),
                    request_blocks: parsed.blocks.clone(),
                    response_blocks: Vec::new(),
                    usage: None,
                    status: ExchangeStatus::Pending,
                    sse_buffer: String::new(),
                    bytes_received: 0,
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
            // Extract the last complete data event from the SSE stream
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
                // No parseable response found in stream, still mark complete
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
fn is_api_endpoint(path: &str) -> bool {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};
    is_messages_path(path) || is_chat_completions_path(path) || is_responses_path(path)
}

/// Extract the final complete response JSON from an SSE stream buffer.
///
/// For Anthropic: looks for `event: message_stop` then finds the last
/// `event: message_delta` with usage data, or reconstructs from content deltas.
///
/// For OpenAI: looks for `data: [DONE]` marker and accumulates content deltas.
fn extract_final_response(sse_buffer: &str, provider: Provider, path: &str) -> Option<Vec<u8>> {
    match provider {
        Provider::Anthropic => extract_anthropic_final_response(sse_buffer),
        Provider::OpenAI => {
            if parser::is_responses_path(path) {
                extract_openai_responses_final_response(sse_buffer)
            } else {
                extract_openai_chat_final_response(sse_buffer)
            }
        }
    }
}

/// Reconstruct an Anthropic response from SSE events.
///
/// Anthropic SSE events for streaming:
/// - `message_start` with `message: {id, model, role, usage: {input_tokens}}`
/// - `content_block_start` with `content_block: {type: "text", text: ""}`
/// - `content_block_delta` with `delta: {type: "text_delta", text: "chunk"}`
/// - `content_block_stop`
/// - `message_delta` with `delta: {stop_reason}, usage: {output_tokens}`
/// - `message_stop`
fn extract_anthropic_final_response(sse_buffer: &str) -> Option<Vec<u8>> {
    let mut model = None;
    let mut accumulated_text = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    for line in sse_buffer.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..]; // Skip "data: "
        if data == "[DONE]" {
            break;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match event_type {
                "message_start" => {
                    if let Some(msg) = json.get("message") {
                        model = msg
                            .get("model")
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string());
                        if let Some(usage) = msg.get("usage") {
                            input_tokens = usage
                                .get("input_tokens")
                                .and_then(|t| t.as_u64())
                                .unwrap_or(0) as u32;
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = json.get("delta") {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                            accumulated_text.push_str(text);
                        }
                    }
                }
                "message_delta" => {
                    if let Some(usage) = json.get("usage") {
                        output_tokens = usage
                            .get("output_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0) as u32;
                    }
                }
                _ => {}
            }
        }
    }

    if accumulated_text.is_empty() && input_tokens == 0 {
        return None;
    }

    // Reconstruct a complete response JSON
    let response = serde_json::json!({
        "model": model,
        "content": [{"type": "text", "text": accumulated_text}],
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    });

    Some(serde_json::to_vec(&response).unwrap_or_default())
}

/// Reconstruct an OpenAI response from SSE events.
///
/// OpenAI SSE events for streaming:
/// - `data: {choices: [{delta: {role: "assistant"}}]}`
/// - `data: {choices: [{delta: {content: "chunk"}}]}`
/// - `data: {choices: [{delta: {}, finish_reason: "stop"}], usage: {...}}`
/// - `data: [DONE]`
fn extract_openai_chat_final_response(sse_buffer: &str) -> Option<Vec<u8>> {
    let mut model = None;
    let mut accumulated_content = String::new();
    let mut prompt_tokens: u32 = 0;
    let mut completion_tokens: u32 = 0;

    for line in sse_buffer.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            if model.is_none() {
                model = json
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
            }

            if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                            accumulated_content.push_str(content);
                        }
                    }
                }
            }

            if let Some(usage) = json.get("usage") {
                prompt_tokens = usage
                    .get("prompt_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
                completion_tokens = usage
                    .get("completion_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
            }
        }
    }

    if accumulated_content.is_empty() && prompt_tokens == 0 {
        return None;
    }

    let response = serde_json::json!({
        "model": model,
        "choices": [{
            "message": {"role": "assistant", "content": accumulated_content}
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    });

    Some(serde_json::to_vec(&response).unwrap_or_default())
}

/// Reconstruct an OpenAI Responses API response from SSE events.
///
/// Handles common event patterns:
/// - `response.output_text.delta` with `delta: "chunk"`
/// - final events that include `usage`
/// - optional full `output` payload in completion events
fn extract_openai_responses_final_response(sse_buffer: &str) -> Option<Vec<u8>> {
    let mut model = None;
    let mut accumulated_text = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    for line in sse_buffer.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            if model.is_none() {
                model = json
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
            }

            // Streaming delta events.
            if json.get("type").and_then(|t| t.as_str()) == Some("response.output_text.delta") {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    accumulated_text.push_str(delta);
                }
            }

            // Some providers include full output arrays in terminal events.
            if accumulated_text.is_empty() {
                if let Some(output) = json.get("output").and_then(|o| o.as_array()) {
                    for item in output {
                        if item.get("type").and_then(|t| t.as_str()) == Some("message") {
                            if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                                for part in content {
                                    let part_type =
                                        part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if (part_type == "output_text" || part_type == "text")
                                        && part.get("text").and_then(|t| t.as_str()).is_some()
                                    {
                                        accumulated_text.push_str(
                                            part.get("text")
                                                .and_then(|t| t.as_str())
                                                .unwrap_or_default(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(usage) = json.get("usage") {
                input_tokens = usage
                    .get("input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
                output_tokens = usage
                    .get("output_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
            }
        }
    }

    if accumulated_text.is_empty() && input_tokens == 0 {
        return None;
    }

    let response = serde_json::json!({
        "model": model,
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": accumulated_text
            }]
        }],
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    });

    Some(serde_json::to_vec(&response).unwrap_or_default())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_anthropic_request() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "system": "Be helpful.",
            "messages": [
                {"role": "user", "content": "Hello!"},
                {"role": "assistant", "content": "Hi there!"}
            ]
        }))
        .unwrap()
    }

    fn sample_anthropic_response() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-20250514",
            "content": [{"type": "text", "text": "I'm doing well!"}],
            "usage": {"input_tokens": 20, "output_tokens": 10}
        }))
        .unwrap()
    }

    #[test]
    fn test_capture_request_stores_exchange() {
        let store = CaptureStore::default();
        let body = sample_anthropic_request();

        let result = store.capture_request("req_1", "/v1/messages", &body);
        assert!(result.is_some());

        let exchange = store.get_exchange("req_1").unwrap();
        assert_eq!(exchange.provider, Provider::Anthropic);
        assert_eq!(exchange.status, ExchangeStatus::Pending);
        assert_eq!(exchange.request_blocks.len(), 3); // system + user + assistant
    }

    #[test]
    fn test_capture_request_non_api_endpoint() {
        let store = CaptureStore::default();
        let result = store.capture_request("req_1", "/health", b"{}");
        assert!(result.is_none());
    }

    #[test]
    fn test_capture_response_completes_exchange() {
        let store = CaptureStore::default();
        let req_body = sample_anthropic_request();
        store.capture_request("req_1", "/v1/messages", &req_body);

        let resp_body = sample_anthropic_response();
        let result = store.capture_response("req_1", 200, &resp_body);
        assert!(result.is_some());

        let exchange = result.unwrap();
        assert_eq!(exchange.status, ExchangeStatus::Complete);
        assert_eq!(exchange.response_blocks.len(), 1);
        assert!(exchange.usage.is_some());
        assert_eq!(exchange.usage.as_ref().unwrap().input_tokens, 20);
    }

    #[test]
    fn test_capture_response_error_status() {
        let store = CaptureStore::default();
        let req_body = sample_anthropic_request();
        store.capture_request("req_1", "/v1/messages", &req_body);

        let result = store.capture_response("req_1", 500, b"Internal Server Error");
        assert!(result.is_some());
        assert_eq!(result.unwrap().status, ExchangeStatus::Failed);
    }

    #[test]
    fn test_sse_chunk_accumulation() {
        let store = CaptureStore::default();
        let req_body = sample_anthropic_request();
        store.capture_request("req_1", "/v1/messages", &req_body);

        let bytes = store.append_sse_chunk("req_1", b"data: {\"type\":\"message_start\"}\n\n");
        assert!(bytes > 0);

        let exchange = store.get_exchange("req_1").unwrap();
        assert_eq!(exchange.status, ExchangeStatus::Streaming);
    }

    #[test]
    fn test_extract_anthropic_sse() {
        let sse = "\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":15}}}\n\
\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world!\"}}\n\
\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\
\n\
data: {\"type\":\"message_stop\"}\n\
\n";

        let result = extract_anthropic_final_response(sse).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(json["content"][0]["text"], "Hello world!");
        assert_eq!(json["usage"]["input_tokens"], 15);
        assert_eq!(json["usage"]["output_tokens"], 5);
    }

    #[test]
    fn test_extract_openai_sse() {
        let sse = "\
data: {\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{\"content\":\"Hi \"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{\"content\":\"there!\"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3,\"total_tokens\":13}}\n\
\n\
data: [DONE]\n\
\n";

        let result = extract_openai_chat_final_response(sse).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(json["choices"][0]["message"]["content"], "Hi there!");
        assert_eq!(json["usage"]["prompt_tokens"], 10);
        assert_eq!(json["usage"]["completion_tokens"], 3);
    }

    #[test]
    fn test_extract_openai_responses_sse() {
        let sse = "\
data: {\"type\":\"response.output_text.delta\",\"model\":\"gpt-4.1\",\"delta\":\"Hi \"}\n\
\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"there!\"}\n\
\n\
data: {\"type\":\"response.completed\",\"usage\":{\"input_tokens\":11,\"output_tokens\":4}}\n\
\n\
data: [DONE]\n\
\n";

        let result = extract_openai_responses_final_response(sse).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(json["output"][0]["content"][0]["text"], "Hi there!");
        assert_eq!(json["usage"]["input_tokens"], 11);
        assert_eq!(json["usage"]["output_tokens"], 4);
    }

    #[test]
    fn test_all_blocks_returns_completed() {
        let store = CaptureStore::default();
        let req_body = sample_anthropic_request();
        store.capture_request("req_1", "/v1/messages", &req_body);

        // Not complete yet — should return empty
        assert!(store.all_blocks().is_empty());

        let resp_body = sample_anthropic_response();
        store.capture_response("req_1", 200, &resp_body);

        let blocks = store.all_blocks();
        assert_eq!(blocks.len(), 4); // 3 request + 1 response
    }

    #[test]
    fn test_is_api_endpoint() {
        assert!(is_api_endpoint("/v1/messages"));
        assert!(is_api_endpoint("/v1/chat/completions"));
        assert!(is_api_endpoint("/v1/responses"));
        // Bare paths (no /v1/ prefix)
        assert!(is_api_endpoint("/messages"));
        assert!(is_api_endpoint("/chat/completions"));
        assert!(is_api_endpoint("/responses"));
        // Non-API paths
        assert!(!is_api_endpoint("/health"));
        assert!(!is_api_endpoint("/v1/models"));
    }
}
