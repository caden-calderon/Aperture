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
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::debug;

use super::ProxyState;
use crate::metacog::{dispatch_tool, ToolOutput};

/// Route an `/_aperture/*` request to the appropriate handler.
///
/// Returns `Some(Response)` if the path matched an internal route,
/// `None` if the path is not an `/_aperture/` route (caller should proxy normally).
pub async fn handle_aperture_route(
    state: &Arc<ProxyState>,
    path: &str,
    req: Request<Body>,
) -> Option<Response> {
    if !path.starts_with("/_aperture/") && path != "/_aperture" {
        return None;
    }

    let sub_path = path.strip_prefix("/_aperture").unwrap_or("");

    Some(match sub_path {
        "/health" => health_check().into_response(),
        "/context/preview" => dispatch_context_tool(state, "aperture_context_preview", req).await,
        "/context/read" => dispatch_context_tool(state, "aperture_context_read", req).await,
        "/context/search" => dispatch_context_tool(state, "aperture_context_search", req).await,
        "/context/plan" => dispatch_context_tool(state, "aperture_context_plan", req).await,
        "/context/status" => dispatch_context_tool(state, "aperture_context_status", req).await,
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown aperture route"})),
        )
            .into_response(),
    })
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
async fn dispatch_context_tool(
    state: &Arc<ProxyState>,
    tool_name: &str,
    req: Request<Body>,
) -> Response {
    let engine = match &state.engine {
        Some(e) => e,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "context engine not available"})),
            )
                .into_response();
        }
    };

    // Parse request body as JSON arguments (empty object if no body)
    let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 64).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid request body: {e}")})),
            )
                .into_response();
        }
    };

    let arguments: Value = if body_bytes.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body_bytes) {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid JSON: {e}")})),
                )
                    .into_response();
            }
        }
    };

    let blocks = engine.active_session_blocks();
    let budget = engine.budget_status();

    debug!(
        tool = tool_name,
        blocks = blocks.len(),
        "Dispatching context tool via HTTP API"
    );

    let output: ToolOutput =
        dispatch_tool(tool_name, &arguments, &blocks, &budget, &engine.planner);

    // Tool errors are application-level (returned in JSON body), not HTTP errors.
    (
        StatusCode::OK,
        Json(json!({
            "content": output.content,
            "is_error": output.is_error
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ContextEngine;
    use crate::proxy::ProxyState;

    fn make_state_with_engine() -> Arc<ProxyState> {
        let engine = Arc::new(ContextEngine::new_in_memory(None));
        let mut state = ProxyState::new().unwrap();
        state.engine = Some(engine);
        Arc::new(state)
    }

    fn make_state_without_engine() -> Arc<ProxyState> {
        Arc::new(ProxyState::new().unwrap())
    }

    fn make_request(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/_aperture/context/preview")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn make_empty_request() -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/_aperture/context/preview")
            .body(Body::empty())
            .unwrap()
    }

    fn make_get_request(path: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_health_check() {
        let state = make_state_with_engine();
        let req = make_get_request("/_aperture/health");
        let resp = handle_aperture_route(&state, "/_aperture/health", req)
            .await
            .expect("should match route");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "aperture");
    }

    #[tokio::test]
    async fn test_non_aperture_path_returns_none() {
        let state = make_state_with_engine();
        let req = make_get_request("/v1/messages");
        let result = handle_aperture_route(&state, "/v1/messages", req).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_unknown_aperture_route_returns_404() {
        let state = make_state_with_engine();
        let req = make_get_request("/_aperture/nonexistent");
        let resp = handle_aperture_route(&state, "/_aperture/nonexistent", req)
            .await
            .expect("should match route");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_context_preview_empty_engine() {
        let state = make_state_with_engine();
        let req = make_empty_request();
        let resp = dispatch_context_tool(&state, "aperture_context_preview", req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["is_error"].as_bool().unwrap());
        assert!(json["content"].as_str().unwrap().contains("0 blocks"));
    }

    #[tokio::test]
    async fn test_engine_unavailable_returns_503() {
        let state = make_state_without_engine();
        let req = make_empty_request();
        let resp = dispatch_context_tool(&state, "aperture_context_preview", req).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_context_read_requires_block_id() {
        let state = make_state_with_engine();
        let req = make_request(r#"{"block_id": ""}"#);
        let resp = dispatch_context_tool(&state, "aperture_context_read", req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["is_error"].as_bool().unwrap());
        assert!(json["content"].as_str().unwrap().contains("required"));
    }

    #[tokio::test]
    async fn test_context_search_empty_query() {
        let state = make_state_with_engine();
        let req = make_request(r#"{"query": ""}"#);
        let resp = dispatch_context_tool(&state, "aperture_context_search", req).await;

        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["is_error"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_invalid_json_body() {
        let state = make_state_with_engine();
        let req = Request::builder()
            .method("POST")
            .uri("/_aperture/context/preview")
            .body(Body::from("not json"))
            .unwrap();
        let resp = dispatch_context_tool(&state, "aperture_context_preview", req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
