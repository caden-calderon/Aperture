use super::*;
use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};
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

fn make_block(id: &str, role: Role, content: &str) -> Block {
    Block {
        id: id.to_string(),
        role,
        block_type: None,
        content: content.to_string(),
        tokens: (content.len() as u32) / 4,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        zone: Zone::BuiltIn(BuiltInZone::Middle),
        pinned: None,
        compression_level: CompressionLevel::Original,
        compressed_versions: CompressionVersions {
            original: CompressionVersion {
                content: content.to_string(),
                tokens: (content.len() as u32) / 4,
            },
            trimmed: None,
            summarized: None,
            minimal: None,
        },
        usage_heat: 0.0,
        position_relevance: 0.0,
        last_referenced_turn: 0,
        reference_count: 0,
        topic_cluster: None,
        topic_keywords: vec![],
        metadata: BlockMetadata {
            provider: "anthropic".to_string(),
            turn_index: 0,
            tool_name: None,
            file_paths: vec![],
        },
    }
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
    assert_eq!(json["session_id"], "__legacy__");
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

#[tokio::test]
async fn test_context_tool_circuit_breaker_blocks_requests() {
    let state = make_state_with_engine();
    for _ in 0..24 {
        let _ = state.runaway_guard.record_context_tool_call();
    }

    let req = make_empty_request();
    let resp = dispatch_context_tool(&state, "aperture_context_preview", req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["is_error"], true);
    assert!(json["content"]
        .as_str()
        .unwrap_or_default()
        .contains("circuit breaker"));
}

#[tokio::test]
async fn test_session_override_uses_requested_session_id() {
    let state = make_state_with_engine();
    let engine = state.engine.as_ref().expect("engine available");

    let session_one = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        Some("thread-one"),
        vec![make_block("u1", Role::User, "one")],
        vec![],
        0,
    );
    let session_two = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        Some("thread-two"),
        vec![make_block("u2", Role::User, "two")],
        vec![],
        0,
    );
    assert_ne!(session_one.session_id, session_two.session_id);

    let req = make_request(&format!(
        r#"{{"_aperture_session_id":"{}"}}"#,
        session_one.session_id
    ));
    let resp = dispatch_context_tool(&state, "aperture_context_preview", req).await;
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["session_id"], session_one.session_id);
    assert!(json["content"]
        .as_str()
        .unwrap_or_default()
        .contains("1 blocks"));
}
