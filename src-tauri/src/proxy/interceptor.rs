//! Context tool interception for non-streaming responses.
//!
//! When the model uses `aperture_context_*` tools in its response, this module:
//! 1. Extracts those calls from the response JSON
//! 2. Dispatches them internally (no round-trip to the model)
//! 3. If ONLY context tools were called: re-invokes upstream with results appended
//! 4. If mixed (real + context tools): strips context calls, returns modified response
//!
//! Re-invoke has a depth limit (max 3) and total timeout (60s) for safety.

use bytes::Bytes;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::engine::ContextEngine;
use crate::metacog::{self, detect_runtime, is_context_tool_name, RuntimeKind};
use crate::proxy::parser::ParsedRequest;
use crate::proxy::ProxyState;

/// Maximum depth for re-invoke loops (prevents infinite recursion).
const MAX_REINVOKE_DEPTH: u32 = 3;

/// Maximum total time for all re-invokes combined.
const REINVOKE_TIMEOUT: Duration = Duration::from_secs(60);

/// Attempt to intercept context tool calls from a non-streaming response.
///
/// Returns `Ok(Some(response))` if context tools were handled (either re-invoked
/// or stripped). Returns `Ok(None)` if no context tools were found. Falls back
/// to `Ok(None)` on any error (fail-open).
#[allow(clippy::too_many_arguments)]
pub async fn try_context_tool_interception(
    state: &Arc<ProxyState>,
    request_id: &str,
    path: &str,
    parsed: &ParsedRequest,
    engine: &ContextEngine,
    response_bytes: &Bytes,
    original_request_body: &[u8],
    upstream_url: &str,
    request_headers: &axum::http::HeaderMap,
    request_start: Instant,
) -> Option<InterceptionResult> {
    // Only intercept for non-passive runtimes on non-streaming requests
    let provider_str = parsed.provider.to_string();
    let runtime_kind = detect_runtime(path, &provider_str);
    if runtime_kind == RuntimeKind::Passive || runtime_kind == RuntimeKind::ClaudeMcp {
        return None;
    }

    let response_json: Value = match serde_json::from_slice(response_bytes) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let runtime = metacog::select_runtime(runtime_kind, path);
    let context_calls = runtime.extract_context_calls(&response_json);

    if context_calls.is_empty() {
        return None;
    }

    debug!(
        request_id = %request_id,
        count = context_calls.len(),
        "Extracted context tool calls from response"
    );

    // Dispatch each context tool call
    let blocks = engine.active_session_blocks();
    let budget = engine.budget_status();
    let results: Vec<_> = context_calls
        .iter()
        .map(|call| {
            debug!(
                request_id = %request_id,
                tool = %call.name,
                "Dispatching context tool"
            );
            let output = metacog::dispatch_tool(
                &call.name,
                &call.arguments,
                &blocks,
                &budget,
                &engine.planner,
            );
            metacog::ContextToolResult {
                tool_use_id: call.id.clone(),
                content: output.content,
                is_error: output.is_error,
            }
        })
        .collect();

    // Determine if response contains ONLY context tool calls
    let is_context_only = is_context_only_response(&response_json, runtime_kind);

    if is_context_only {
        debug!(
            request_id = %request_id,
            "Context-only response — attempting re-invoke"
        );
        // Re-invoke: build new request with tool results, forward to upstream
        match reinvoke_with_results(
            state,
            request_id,
            path,
            parsed,
            engine,
            original_request_body,
            &response_json,
            &results,
            upstream_url,
            request_headers,
            runtime_kind,
            request_start,
            0,
        )
        .await
        {
            Some(result) => Some(result),
            None => {
                // Re-invoke failed — return stripped response as fallback
                let stripped = strip_context_calls_from_response(response_json, runtime_kind);
                Some(InterceptionResult::ModifiedResponse(
                    serde_json::to_vec(&stripped).unwrap_or_else(|_| response_bytes.to_vec()),
                ))
            }
        }
    } else {
        debug!(
            request_id = %request_id,
            "Mixed response — stripping context tool calls"
        );
        // Mixed: strip context tool calls from response, return to client
        let stripped = strip_context_calls_from_response(response_json, runtime_kind);
        Some(InterceptionResult::ModifiedResponse(
            serde_json::to_vec(&stripped).unwrap_or_else(|_| response_bytes.to_vec()),
        ))
    }
}

/// Result of a context tool interception.
pub enum InterceptionResult {
    /// Response body was modified (context tools stripped or re-invoke result).
    ModifiedResponse(Vec<u8>),
}

/// Check if a response contains ONLY context tool calls (no text, no real tools).
fn is_context_only_response(response_json: &Value, runtime_kind: RuntimeKind) -> bool {
    match runtime_kind {
        RuntimeKind::CodexProxy => {
            // Check if the path indicates Responses or ChatCompletions format
            // by looking at response structure
            if let Some(output) = response_json.get("output").and_then(|o| o.as_array()) {
                // Responses API format
                output.iter().all(|item| {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match item_type {
                        "function_call" => {
                            let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            is_context_tool_name(name)
                        }
                        _ => false,
                    }
                }) && !output.is_empty()
            } else if let Some(choices) = response_json.get("choices").and_then(|c| c.as_array()) {
                // ChatCompletions format
                choices.iter().all(|choice| {
                    let msg = match choice.get("message") {
                        Some(m) => m,
                        None => return false,
                    };
                    // No text content
                    let has_text = msg
                        .get("content")
                        .and_then(|c| c.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false);
                    if has_text {
                        return false;
                    }
                    // All tool calls are context tools
                    match msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                        Some(tcs) if !tcs.is_empty() => tcs.iter().all(|tc| {
                            tc.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                                .map(is_context_tool_name)
                                .unwrap_or(false)
                        }),
                        _ => false,
                    }
                })
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Strip context tool calls from a response, keeping real tool calls and text.
fn strip_context_calls_from_response(mut response_json: Value, runtime_kind: RuntimeKind) -> Value {
    if runtime_kind == RuntimeKind::CodexProxy {
        if let Some(output) = response_json
            .get_mut("output")
            .and_then(|o| o.as_array_mut())
        {
            // Responses API: remove function_call items that are context tools
            output.retain(|item| {
                let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if item_type == "function_call" {
                    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    !is_context_tool_name(name)
                } else {
                    true
                }
            });
        } else if let Some(choices) = response_json
            .get_mut("choices")
            .and_then(|c| c.as_array_mut())
        {
            // ChatCompletions: remove context tool_calls from each choice
            for choice in choices.iter_mut() {
                if let Some(msg) = choice.get_mut("message") {
                    if let Some(tcs) = msg.get_mut("tool_calls").and_then(|t| t.as_array_mut()) {
                        tcs.retain(|tc| {
                            tc.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                                .map(|name| !is_context_tool_name(name))
                                .unwrap_or(true)
                        });
                        // If no tool_calls remain, remove the key
                        if tcs.is_empty() {
                            msg.as_object_mut().map(|obj| obj.remove("tool_calls"));
                        }
                    }
                }
            }
        }
    }
    response_json
}

/// Re-invoke the upstream with context tool results appended.
///
/// Builds a new request from the original body + assistant response + tool results,
/// runs it through the rewriter, and forwards to upstream. Recurses if the new
/// response also contains only context tool calls (up to MAX_REINVOKE_DEPTH).
#[allow(clippy::too_many_arguments)]
async fn reinvoke_with_results(
    state: &Arc<ProxyState>,
    request_id: &str,
    path: &str,
    parsed: &ParsedRequest,
    engine: &ContextEngine,
    original_request_body: &[u8],
    assistant_response: &Value,
    results: &[metacog::ContextToolResult],
    upstream_url: &str,
    request_headers: &axum::http::HeaderMap,
    runtime_kind: RuntimeKind,
    request_start: Instant,
    depth: u32,
) -> Option<InterceptionResult> {
    if depth >= MAX_REINVOKE_DEPTH {
        warn!(
            request_id = %request_id,
            depth,
            "Re-invoke depth limit reached"
        );
        return None;
    }

    if request_start.elapsed() > REINVOKE_TIMEOUT {
        warn!(
            request_id = %request_id,
            elapsed_ms = request_start.elapsed().as_millis() as u64,
            "Re-invoke timeout exceeded"
        );
        return None;
    }

    // Build new request body: original + assistant message + tool results
    let mut new_body: Value = match serde_json::from_slice(original_request_body) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to parse original request for re-invoke: {e}");
            return None;
        }
    };

    let runtime = metacog::select_runtime(runtime_kind, path);

    // Append assistant response to conversation
    append_assistant_response(&mut new_body, assistant_response, runtime_kind);

    // Inject tool results
    runtime.inject_results(&mut new_body, results);

    let new_body_bytes = match serde_json::to_vec(&new_body) {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to serialize re-invoke body: {e}");
            return None;
        }
    };

    debug!(
        request_id = %request_id,
        depth,
        body_bytes = new_body_bytes.len(),
        "Sending re-invoke request"
    );

    // Forward to upstream, carrying over auth headers from original request
    let mut upstream_req = state
        .client
        .request(reqwest::Method::POST, upstream_url)
        .header("content-type", "application/json");

    // Forward auth-relevant headers
    for key in &[
        axum::http::header::AUTHORIZATION,
        axum::http::header::HeaderName::from_static("x-api-key"),
        axum::http::header::HeaderName::from_static("anthropic-version"),
    ] {
        if let Some(val) = request_headers.get(key) {
            upstream_req = upstream_req.header(key.as_str(), val.as_bytes());
        }
    }

    let upstream_response = match upstream_req.body(new_body_bytes.clone()).send().await {
        Ok(resp) => resp,
        Err(e) => {
            warn!("Re-invoke upstream request failed: {e}");
            return None;
        }
    };

    let status = upstream_response.status();
    if !status.is_success() {
        warn!(
            request_id = %request_id,
            status = %status,
            "Re-invoke got non-success status"
        );
        return None;
    }

    let response_bytes = match upstream_response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to read re-invoke response: {e}");
            return None;
        }
    };

    // Check if the new response also has context-only tool calls
    let response_json: Value = match serde_json::from_slice(&response_bytes) {
        Ok(v) => v,
        Err(_) => {
            // Can't parse — just return as-is
            return Some(InterceptionResult::ModifiedResponse(
                response_bytes.to_vec(),
            ));
        }
    };

    let new_context_calls = runtime.extract_context_calls(&response_json);
    if new_context_calls.is_empty() {
        // No more context calls — return this response
        return Some(InterceptionResult::ModifiedResponse(
            response_bytes.to_vec(),
        ));
    }

    // Dispatch new context tool calls
    let blocks = engine.active_session_blocks();
    let budget = engine.budget_status();
    let new_results: Vec<_> = new_context_calls
        .iter()
        .map(|call| {
            let output = metacog::dispatch_tool(
                &call.name,
                &call.arguments,
                &blocks,
                &budget,
                &engine.planner,
            );
            metacog::ContextToolResult {
                tool_use_id: call.id.clone(),
                content: output.content,
                is_error: output.is_error,
            }
        })
        .collect();

    if is_context_only_response(&response_json, runtime_kind) {
        // Recurse with increased depth
        Box::pin(reinvoke_with_results(
            state,
            request_id,
            path,
            parsed,
            engine,
            &new_body_bytes,
            &response_json,
            &new_results,
            upstream_url,
            request_headers,
            runtime_kind,
            request_start,
            depth + 1,
        ))
        .await
    } else {
        // Mixed — strip and return
        let stripped = strip_context_calls_from_response(response_json, runtime_kind);
        Some(InterceptionResult::ModifiedResponse(
            serde_json::to_vec(&stripped).unwrap_or_else(|_| response_bytes.to_vec()),
        ))
    }
}

/// Append the assistant's response to the request body for re-invocation.
fn append_assistant_response(
    request_json: &mut Value,
    response_json: &Value,
    runtime_kind: RuntimeKind,
) {
    if runtime_kind != RuntimeKind::CodexProxy {
        return;
    }

    if let Some(output) = response_json.get("output").and_then(|o| o.as_array()) {
        // Responses API: append output items to input[]
        if let Some(input) = request_json.get_mut("input").and_then(|i| i.as_array_mut()) {
            for item in output {
                input.push(item.clone());
            }
        }
    } else if let Some(choices) = response_json.get("choices").and_then(|c| c.as_array()) {
        // ChatCompletions: append assistant message to messages[]
        if let Some(messages) = request_json
            .get_mut("messages")
            .and_then(|m| m.as_array_mut())
        {
            if let Some(choice) = choices.first() {
                if let Some(msg) = choice.get("message") {
                    messages.push(msg.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{parser::parse_request, ProxyState};
    use axum::http::HeaderMap;
    use serde_json::json;

    // ── is_context_only_response tests ────────────────────────

    #[test]
    fn test_context_only_responses_api() {
        let response = json!({
            "output": [
                {
                    "type": "function_call",
                    "name": "aperture_context_preview",
                    "call_id": "fc_1",
                    "arguments": "{}"
                }
            ]
        });
        assert!(is_context_only_response(&response, RuntimeKind::CodexProxy));
    }

    #[test]
    fn test_mixed_responses_api() {
        let response = json!({
            "output": [
                {
                    "type": "function_call",
                    "name": "aperture_context_preview",
                    "call_id": "fc_1",
                    "arguments": "{}"
                },
                {
                    "type": "function_call",
                    "name": "read_file",
                    "call_id": "fc_2",
                    "arguments": "{\"path\": \"foo.rs\"}"
                }
            ]
        });
        assert!(!is_context_only_response(
            &response,
            RuntimeKind::CodexProxy
        ));
    }

    #[test]
    fn test_context_only_chat_completions() {
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "aperture_context_preview",
                                "arguments": "{}"
                            }
                        }
                    ]
                }
            }]
        });
        assert!(is_context_only_response(&response, RuntimeKind::CodexProxy));
    }

    #[test]
    fn test_mixed_chat_completions_with_text() {
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Let me check...",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "aperture_context_preview",
                                "arguments": "{}"
                            }
                        }
                    ]
                }
            }]
        });
        assert!(!is_context_only_response(
            &response,
            RuntimeKind::CodexProxy
        ));
    }

    #[test]
    fn test_mixed_chat_completions_real_and_context_tools() {
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "aperture_context_preview",
                                "arguments": "{}"
                            }
                        },
                        {
                            "id": "call_2",
                            "type": "function",
                            "function": {
                                "name": "read_file",
                                "arguments": "{}"
                            }
                        }
                    ]
                }
            }]
        });
        assert!(!is_context_only_response(
            &response,
            RuntimeKind::CodexProxy
        ));
    }

    #[test]
    fn test_no_tool_calls_not_context_only() {
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello!"
                }
            }]
        });
        assert!(!is_context_only_response(
            &response,
            RuntimeKind::CodexProxy
        ));
    }

    #[test]
    fn test_passive_runtime_never_context_only() {
        let response = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{"id": "1", "type": "function", "function": {"name": "aperture_context_preview", "arguments": "{}"}}]
                }
            }]
        });
        assert!(!is_context_only_response(&response, RuntimeKind::Passive));
    }

    // ── strip_context_calls_from_response tests ──────────────

    #[test]
    fn test_strip_responses_api() {
        let response = json!({
            "output": [
                {"type": "function_call", "name": "aperture_context_preview", "call_id": "fc_1", "arguments": "{}"},
                {"type": "function_call", "name": "read_file", "call_id": "fc_2", "arguments": "{}"},
                {"type": "message", "content": [{"type": "output_text", "text": "Done"}]}
            ]
        });
        let stripped = strip_context_calls_from_response(response, RuntimeKind::CodexProxy);
        let output = stripped["output"].as_array().unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["name"], "read_file");
        assert_eq!(output[1]["type"], "message");
    }

    #[test]
    fn test_strip_chat_completions() {
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "aperture_context_preview", "arguments": "{}"}},
                        {"id": "call_2", "type": "function", "function": {"name": "read_file", "arguments": "{}"}}
                    ]
                }
            }]
        });
        let stripped = strip_context_calls_from_response(response, RuntimeKind::CodexProxy);
        let tcs = stripped["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0]["function"]["name"], "read_file");
    }

    #[test]
    fn test_strip_all_context_calls_removes_tool_calls_key() {
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "text",
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "aperture_context_preview", "arguments": "{}"}}
                    ]
                }
            }]
        });
        let stripped = strip_context_calls_from_response(response, RuntimeKind::CodexProxy);
        assert!(stripped["choices"][0]["message"]
            .get("tool_calls")
            .is_none());
    }

    // ── append_assistant_response tests ──────────────────────

    #[test]
    fn test_append_assistant_response_responses_api() {
        let mut request = json!({
            "input": [
                {"type": "message", "role": "user", "content": "Hello"}
            ]
        });
        let response = json!({
            "output": [
                {"type": "function_call", "name": "aperture_context_preview", "call_id": "fc_1", "arguments": "{}"}
            ]
        });
        append_assistant_response(&mut request, &response, RuntimeKind::CodexProxy);
        let input = request["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["type"], "function_call");
    }

    #[test]
    fn test_append_assistant_response_chat_completions() {
        let mut request = json!({
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "aperture_context_preview", "arguments": "{}"}}]
                }
            }]
        });
        append_assistant_response(&mut request, &response, RuntimeKind::CodexProxy);
        let messages = request["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "assistant");
    }

    #[tokio::test]
    async fn test_reinvoke_depth_limit_fail_open_returns_none() {
        let state = Arc::new(ProxyState::new().expect("proxy state"));
        let engine = ContextEngine::new_in_memory(None);
        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-4.1",
            "input": [{ "type": "message", "role": "user", "content": "hi" }]
        }))
        .expect("serialize request");
        let parsed = parse_request("/v1/responses", &request_body).expect("parsed request");
        let headers = HeaderMap::new();
        let results: Vec<metacog::ContextToolResult> = vec![];

        let result = reinvoke_with_results(
            &state,
            "req_depth",
            "/v1/responses",
            &parsed,
            &engine,
            &request_body,
            &json!({}),
            &results,
            "http://127.0.0.1:9/v1/responses",
            &headers,
            RuntimeKind::CodexProxy,
            Instant::now(),
            MAX_REINVOKE_DEPTH,
        )
        .await;

        assert!(
            result.is_none(),
            "Depth-limit path should fail-open to caller fallback"
        );
    }

    #[tokio::test]
    async fn test_reinvoke_timeout_fail_open_returns_none() {
        let state = Arc::new(ProxyState::new().expect("proxy state"));
        let engine = ContextEngine::new_in_memory(None);
        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-4.1",
            "input": [{ "type": "message", "role": "user", "content": "hi" }]
        }))
        .expect("serialize request");
        let parsed = parse_request("/v1/responses", &request_body).expect("parsed request");
        let headers = HeaderMap::new();
        let results: Vec<metacog::ContextToolResult> = vec![];
        let timed_out_start = Instant::now() - REINVOKE_TIMEOUT - Duration::from_secs(1);

        let result = reinvoke_with_results(
            &state,
            "req_timeout",
            "/v1/responses",
            &parsed,
            &engine,
            &request_body,
            &json!({}),
            &results,
            "http://127.0.0.1:9/v1/responses",
            &headers,
            RuntimeKind::CodexProxy,
            timed_out_start,
            0,
        )
        .await;

        assert!(
            result.is_none(),
            "Timeout path should fail-open to caller fallback"
        );
    }
}
