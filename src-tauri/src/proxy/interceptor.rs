//! Context tool interception for non-streaming responses.
//!
//! When the model uses `aperture_context_*` tools in its response, this module:
//! 1. Extracts those calls from the response JSON
//! 2. Dispatches them internally (no round-trip to the model)
//! 3. If only context tools were called: re-invokes upstream with results appended
//! 4. If mixed (real + context tools): strips context calls, returns modified response
//!
//! Re-invoke has a depth limit (max 3) and total timeout (60s) for safety.

mod response;
#[cfg(test)]
mod tests;

use bytes::Bytes;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use self::response::{
    append_assistant_response, build_circuit_breaker_response, is_context_only_response,
    strip_context_calls_from_response,
};
use crate::engine::ContextEngine;
use crate::metacog::{self, detect_runtime, RuntimeKind};
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

    if let Some(remaining) = state.runaway_guard.context_circuit_remaining() {
        return Some(InterceptionResult::ModifiedResponse(
            build_circuit_breaker_response(runtime_kind, &response_json, remaining.as_secs()),
        ));
    }

    debug!(
        request_id = %request_id,
        count = context_calls.len(),
        "Extracted context tool calls from response"
    );

    // Dispatch each context tool call.
    let session_id = engine.resolve_session(
        &provider_str,
        &parsed.model,
        "proxy",
        parsed.thread_identity.as_deref(),
    );
    let blocks = engine.session_blocks(&session_id);
    let budget = engine.session_budget_status(&session_id);
    let mut results = Vec::with_capacity(context_calls.len());
    for call in &context_calls {
        let tripped = state
            .runaway_guard
            .record_context_tool_call()
            .map(|alert| alert.hard_limit)
            .unwrap_or(false);
        if tripped {
            let wait_secs = state
                .runaway_guard
                .context_circuit_remaining()
                .map(|d| d.as_secs())
                .unwrap_or(30);
            return Some(InterceptionResult::ModifiedResponse(
                build_circuit_breaker_response(runtime_kind, &response_json, wait_secs),
            ));
        }

        debug!(
            request_id = %request_id,
            tool = %call.name,
            "Dispatching context tool"
        );
        let output = metacog::dispatch_tool_for_session(
            &call.name,
            &call.arguments,
            &blocks,
            &budget,
            &engine.planner,
            &session_id,
        );
        results.push(metacog::ContextToolResult {
            tool_use_id: call.id.clone(),
            content: output.content,
            is_error: output.is_error,
        });
    }

    // Determine if response contains only context tool calls.
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
    let provider_str = parsed.provider.to_string();
    let session_id = engine.resolve_session(
        &provider_str,
        &parsed.model,
        "proxy",
        parsed.thread_identity.as_deref(),
    );
    let blocks = engine.session_blocks(&session_id);
    let budget = engine.session_budget_status(&session_id);
    let new_results: Vec<_> = new_context_calls
        .iter()
        .map(|call| {
            let output = metacog::dispatch_tool_for_session(
                &call.name,
                &call.arguments,
                &blocks,
                &budget,
                &engine.planner,
                &session_id,
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
