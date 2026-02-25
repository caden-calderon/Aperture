//! Internal HTTP API for Aperture context tools.
//!
//! Routes under `/_aperture/` are intercepted before normal proxy routing
//! and handled locally. The MCP server binary calls these endpoints over HTTP.
//!
//! Endpoints:
//! - `POST /_aperture/context/preview` — Block inventory with smart previews
//! - `POST /_aperture/context/read`    — Full content of a single block
//! - `POST /_aperture/context/search`  — Search across blocks
//! - `POST /_aperture/context/plan`    — Plan context changes
//! - `POST /_aperture/context/status`  — Full manifest
//! - `GET  /_aperture/health`          — Health check

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::{debug, warn};

use super::ProxyState;
use crate::engine::ContextEngine;
use crate::events::types::ApertureEvent;
use crate::metacog::{
    context_tools_passive_only, dispatch_tool_with_limits_for_session, ToolOutput, ToolOutputLimits,
};


const CONTEXT_API_BODY_LIMIT_BYTES: usize = 1024 * 64;

/// Route an `/_aperture/*` request to the appropriate handler.
///
/// Returns `Some(Response)` if the path matched an internal route,
/// `None` if the path is not an `/_aperture/` route (caller should proxy normally).
pub async fn handle_aperture_route(
    state: &Arc<ProxyState>,
    path: &str,
    req: Request<Body>,
) -> Option<Response> {
    if !is_aperture_path(path) {
        return None;
    }

    // Handle CORS preflight for browser/webview access.
    if req.method() == Method::OPTIONS {
        return Some(cors_preflight_response());
    }

    let sub_path = path.strip_prefix("/_aperture").unwrap_or("");

    let response = if sub_path == "/health" {
        health_check().into_response()
    } else if sub_path == "/events" {
        sse_events(state).into_response()
    } else if let Some(command) = sub_path.strip_prefix("/ipc/").filter(|c| !c.is_empty()) {
        super::ipc_api::handle_ipc_command(state, command, req).await
    } else if let Some(tool_name) = context_tool_name(sub_path) {
        dispatch_context_tool(state, tool_name, req).await
    } else {
        json_error_response(StatusCode::NOT_FOUND, "unknown aperture route")
    };

    Some(with_cors_headers(response))
}

fn is_aperture_path(path: &str) -> bool {
    path.starts_with("/_aperture/") || path == "/_aperture"
}

/// CORS preflight response for `/_aperture/` endpoints.
///
/// The Tauri webview makes cross-origin requests from `tauri://localhost`
/// or `http://localhost:1420` (Vite dev) to the proxy at `127.0.0.1:5400`.
fn cors_preflight_response() -> Response {
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let headers = resp.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS".parse().unwrap());
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, "content-type".parse().unwrap());
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, "86400".parse().unwrap());
    resp
}

/// Add CORS headers to an aperture API response.
fn with_cors_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    response
}

fn context_tool_name(sub_path: &str) -> Option<&'static str> {
    match sub_path {
        "/context/preview" => Some("aperture_context_preview"),
        "/context/read" => Some("aperture_context_read"),
        "/context/search" => Some("aperture_context_search"),
        "/context/plan" => Some("aperture_context_plan"),
        "/context/status" => Some("aperture_context_status"),
        _ => None,
    }
}

/// Health check — confirms the proxy is running and engine is available.
fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "aperture",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Dispatch a context tool via the engine, returning a JSON response.
///
/// Serialized by `aperture_semaphore` — only one `/_aperture/context/*`
/// request runs at a time to prevent concurrent engine access crashes.
async fn dispatch_context_tool(
    state: &Arc<ProxyState>,
    tool_name: &str,
    req: Request<Body>,
) -> Response {
    // Serialize all context tool requests through a semaphore to prevent
    // concurrent engine access that can crash the proxy (R14 bug).
    let _permit = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.aperture_semaphore.acquire(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => {
            return json_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "context engine semaphore closed",
            )
        }
        Err(_) => {
            return json_error_response(
                StatusCode::GATEWAY_TIMEOUT,
                "context tool request timed out waiting for engine",
            )
        }
    };

    if context_tools_passive_only() {
        return context_tools_disabled_response(
            "Aperture context tools are disabled (passive-only mode). Set APERTURE_CONTEXT_TOOLS_MODE=enabled to re-enable.",
        );
    }

    let engine = match state.engine.as_ref() {
        Some(engine) => engine,
        None => {
            return json_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "context engine not available",
            )
        }
    };

    let mut arguments = match parse_tool_arguments(req).await {
        Ok(arguments) => arguments,
        Err(response) => return response,
    };

    let session_id = resolve_tool_session_id(engine, &mut arguments);
    let blocks = engine.session_blocks(&session_id);
    let budget = engine.session_budget_status(&session_id);

    if let Some(remaining) = state.runaway_guard.context_circuit_remaining() {
        let secs = remaining.as_secs().max(1);
        return context_tool_error_response(&format!(
            "Aperture context tools are temporarily blocked by circuit breaker due to runaway call rate. Retry in ~{}s.",
            secs
        ));
    }

    let runaway_alert = state.runaway_guard.record_context_tool_call();
    if let Some(alert) = &runaway_alert {
        warn!(
            tool = tool_name,
            channel = alert.channel,
            count = alert.count,
            threshold = alert.threshold,
            window_secs = alert.window_secs,
            hard_limit = alert.hard_limit,
            "Runaway guardrail warning: high context-tool call rate"
        );
        if let Some(ref dispatcher) = state.dispatcher {
            dispatcher.proxy_error(
                None,
                &format!(
                    "Guardrail warning: high context-tool call rate ({} calls in {}s).",
                    alert.count, alert.window_secs
                ),
            );
        }
    }

    if runaway_alert
        .as_ref()
        .map(|alert| alert.hard_limit)
        .unwrap_or(false)
    {
        let pct = (budget.utilization * 100.0).round() as u32;
        return context_tool_error_response(&format!(
            "Aperture circuit breaker triggered after sustained context-tool burst. Context tools are now blocked briefly to stop runaway token burn. Current context: {} blocks, {}% budget.",
            blocks.len(),
            pct
        ));
    }

    // Diagnostic tracing (R9-1): log session ID on plan commit to detect H1 mismatch.
    if tool_name == "aperture_context_plan" {
        let plan_op = arguments
            .get("control")
            .and_then(|c| c.get("op"))
            .and_then(|v| v.as_str())
            .unwrap_or("(implicit)");
        if plan_op == "commit" || plan_op == "stage" {
            warn!(
                tool = tool_name,
                op = plan_op,
                session_id = %session_id,
                blocks = blocks.len(),
                "R9-DIAG context_api: plan operation with resolved session"
            );
        }
    }

    debug!(
        tool = tool_name,
        blocks = blocks.len(),
        "Dispatching context tool via HTTP API"
    );

    let output: ToolOutput = dispatch_tool_with_limits_for_session(
        tool_name,
        &arguments,
        &blocks,
        &budget,
        &engine.planner,
        ToolOutputLimits::default(),
        &session_id,
    );

    // Tool errors are application-level (returned in JSON body), not HTTP errors.
    (
        StatusCode::OK,
        Json(json!({
            "content": output.content,
            "is_error": output.is_error,
            "session_id": session_id
        })),
    )
        .into_response()
}

/// SSE event stream endpoint.
///
/// Returns an infinite SSE stream of `ApertureEvent` JSON payloads.
/// High-frequency `ResponseStreaming` events use a named `stream-progress` event type
/// so clients can filter efficiently.
fn sse_events(state: &Arc<ProxyState>) -> Response {
    let broadcaster = match &state.broadcaster {
        Some(b) => b.clone(),
        None => {
            return json_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "SSE events not available (running inside Tauri — use Tauri event listeners instead)",
            )
        }
    };

    let rx = broadcaster.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let json = serde_json::to_string(&event).ok()?;
            let sse = match &event {
                ApertureEvent::ResponseStreaming { .. } => {
                    SseEvent::default().event("stream-progress").data(json)
                }
                _ => SseEvent::default().data(json),
            };
            Some(Ok::<_, Infallible>(sse))
        }
        Err(_) => None, // Lagged — skip missed events
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn parse_tool_arguments(req: Request<Body>) -> Result<Value, Response> {
    let body_bytes = axum::body::to_bytes(req.into_body(), CONTEXT_API_BODY_LIMIT_BYTES)
        .await
        .map_err(|error| {
            json_error_response(
                StatusCode::BAD_REQUEST,
                &format!("invalid request body: {error}"),
            )
        })?;

    if body_bytes.is_empty() {
        return Ok(json!({}));
    }

    serde_json::from_slice(&body_bytes).map_err(|error| {
        json_error_response(StatusCode::BAD_REQUEST, &format!("invalid JSON: {error}"))
    })
}

fn resolve_tool_session_id(engine: &ContextEngine, arguments: &mut Value) -> String {
    let requested = arguments
        .as_object()
        .and_then(|obj| obj.get("_aperture_session_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    if let Some(obj) = arguments.as_object_mut() {
        obj.remove("_aperture_session_id");
    }

    if let Some(requested_id) = requested {
        if engine.has_session(&requested_id) {
            return requested_id;
        }
    }

    engine
        .active_session_id()
        .unwrap_or_else(|| "__legacy__".to_string())
}

fn json_error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn context_tool_error_response(message: &str) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "content": message,
            "is_error": true
        })),
    )
        .into_response()
}

fn context_tools_disabled_response(message: &str) -> Response {
    context_tool_error_response(message)
}

#[cfg(test)]
mod tests;
