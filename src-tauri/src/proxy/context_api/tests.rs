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

// ── Concurrent MCP Request Tests (R14 crash fix) ──────────────────────

/// Seed an engine with N blocks across a session so tools have data to query.
fn seed_engine_with_blocks(engine: &crate::engine::ContextEngine, count: usize) {
    let blocks: Vec<Block> = (0..count)
        .map(|i| make_block(&format!("b{i}"), Role::User, &format!("Block {i} content with some searchable keywords like aperture context engine planner")))
        .collect();
    engine.ingest("anthropic", "claude-sonnet-4-6", "proxy", Some("stress-test"), blocks, vec![], 0);
}

/// Helper to build a search request with a query.
fn make_search_request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/_aperture/context/search")
        .header("content-type", "application/json")
        .body(Body::from(format!(r#"{{"query": "{query}"}}"#)))
        .unwrap()
}

#[tokio::test]
async fn test_concurrent_search_requests_serialized() {
    let state = make_state_with_engine();
    seed_engine_with_blocks(state.engine.as_ref().unwrap(), 50);

    // Fire 5 parallel search requests — the exact pattern that crashed R14.
    let mut handles = vec![];
    for i in 0..5 {
        let state = state.clone();
        handles.push(tokio::spawn(async move {
            let req = make_search_request(&format!("Block {i}"));
            dispatch_context_tool(&state, "aperture_context_search", req).await
        }));
    }

    let mut success_count = 0;
    for handle in handles {
        let resp = handle.await.expect("task should not panic");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["is_error"].as_bool().unwrap_or(true));
        success_count += 1;
    }
    assert_eq!(success_count, 5);
}

#[tokio::test]
async fn test_concurrent_mixed_tool_requests() {
    let state = make_state_with_engine();
    seed_engine_with_blocks(state.engine.as_ref().unwrap(), 30);

    // Fire preview + search + status + search + preview in parallel.
    let tools: Vec<(&str, &str)> = vec![
        ("aperture_context_preview", "{}"),
        ("aperture_context_search", r#"{"query": "aperture"}"#),
        ("aperture_context_status", "{}"),
        ("aperture_context_search", r#"{"query": "planner"}"#),
        ("aperture_context_preview", "{}"),
    ];

    let mut handles = vec![];
    for (tool_name, body) in tools {
        let state = state.clone();
        let body = body.to_string();
        let tool = tool_name.to_string();
        handles.push(tokio::spawn(async move {
            let req = Request::builder()
                .method("POST")
                .uri(format!("/_aperture/context/{}", tool.strip_prefix("aperture_context_").unwrap()))
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap();
            dispatch_context_tool(&Arc::new((*state).clone()), &tool, req).await
        }));
    }

    for handle in handles {
        let resp = handle.await.expect("task should not panic");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["is_error"].as_bool().unwrap_or(true), "tool returned error: {}", json["content"]);
    }
}

#[tokio::test]
async fn test_semaphore_serializes_requests() {
    // Verify that the semaphore actually serializes — acquire it manually,
    // then fire a request which must wait. Release, then it should complete.
    let state = make_state_with_engine();
    seed_engine_with_blocks(state.engine.as_ref().unwrap(), 5);

    // Acquire the semaphore manually.
    let permit = state.aperture_semaphore.acquire().await.unwrap();

    // Fire a request that will block on the semaphore.
    let state_clone = state.clone();
    let handle = tokio::spawn(async move {
        let req = make_empty_request();
        dispatch_context_tool(&state_clone, "aperture_context_preview", req).await
    });

    // Give the spawned task time to start and block on semaphore.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!handle.is_finished(), "request should be blocked by held semaphore");

    // Release the semaphore — request should now complete.
    drop(permit);

    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("should complete within timeout")
        .expect("task should not panic");
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_concurrent_plan_stage_and_search() {
    // Simulate the R14 scenario: plan operations + search running concurrently.
    let state = make_state_with_engine();
    seed_engine_with_blocks(state.engine.as_ref().unwrap(), 20);

    let state1 = state.clone();
    let handle_plan = tokio::spawn(async move {
        let body = r#"{"control": {"op": "preview"}}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/_aperture/context/plan")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        dispatch_context_tool(&state1, "aperture_context_plan", req).await
    });

    let state2 = state.clone();
    let handle_search = tokio::spawn(async move {
        let req = make_search_request("engine");
        dispatch_context_tool(&state2, "aperture_context_search", req).await
    });

    let resp_plan = handle_plan.await.expect("plan task should not panic");
    let resp_search = handle_search.await.expect("search task should not panic");

    assert_eq!(resp_plan.status(), StatusCode::OK);
    assert_eq!(resp_search.status(), StatusCode::OK);
}
