//! Integration tests for the proxy flow.
//!
//! These tests start an actual proxy server, send requests through it,
//! and verify capture, parsing, and hot-patch behavior.
//!
//! Uses a mock upstream server (another axum instance) to avoid real API calls.

use aperture_lib::engine::types::Role;
use aperture_lib::proxy::handler::proxy_handler;
use aperture_lib::proxy::hot_patch::{HotPatch, HotPatchQueue, PatchSource};
use aperture_lib::proxy::{ProxyState, UpstreamConfig};
use axum::{
    extract::OriginalUri, http::StatusCode, response::IntoResponse, routing::post, Json, Router,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;

/// Spin up a mock upstream that echoes back the request body as a JSON response.
async fn start_mock_upstream() -> (String, u16) {
    let app = Router::new()
        .route(
            "/v1/messages",
            post(|Json(body): Json<serde_json::Value>| async move {
                // Return an Anthropic-style response echoing the request
                Json(serde_json::json!({
                    "id": "msg_test",
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-3-haiku-20240307",
                    "content": [{ "type": "text", "text": format!("Echo: {}", body) }],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }))
            }),
        )
        .route(
            "/v1/chat/completions",
            post(|Json(body): Json<serde_json::Value>| async move {
                Json(serde_json::json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "model": "gpt-4",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": format!("Echo: {}", body) },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
                }))
            }),
        )
        .route(
            "/v1/responses",
            post(
                |OriginalUri(uri): OriginalUri, Json(body): Json<serde_json::Value>| async move {
                    let query = uri.query().unwrap_or_default();
                    let require_trace = body
                        .get("require_trace")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if require_trace && !query.contains("trace=1") {
                        return (
                            StatusCode::BAD_REQUEST,
                            "missing required trace query parameter",
                        )
                            .into_response();
                    }

                    let stream = body
                        .get("stream")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if stream {
                        // Minimal OpenAI-style SSE stream, finite and closed.
                        let sse = concat!(
                            "data: {\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
                            "data: {\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
                            "data: [DONE]\n\n",
                        );
                        return (
                            StatusCode::OK,
                            [("content-type", "text/event-stream")],
                            sse,
                        )
                            .into_response();
                    }

                    Json(serde_json::json!({
                        "id": "resp_test",
                        "model": "gpt-4.1",
                        "output": [{
                            "type": "message",
                            "content": [{ "type": "output_text", "text": "Echo from responses" }]
                        }],
                        "usage": { "input_tokens": 12, "output_tokens": 6 }
                    }))
                    .into_response()
                },
            ),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Small delay to let the server bind
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (addr, port)
}

/// Start the Aperture proxy pointed at a mock upstream.
async fn start_test_proxy(
    upstream_url: &str,
    hot_patches: Option<Arc<HotPatchQueue>>,
) -> (u16, Arc<ProxyState>) {
    let config = UpstreamConfig {
        anthropic_url: upstream_url.to_string(),
        openai_url: upstream_url.to_string(),
    };

    let mut state = ProxyState::with_config(config).unwrap();
    if let Some(hp) = hot_patches {
        state.hot_patches = hp;
    }
    let state = Arc::new(state);

    let app = Router::new()
        .route("/{*path}", axum::routing::any(proxy_handler))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (port, state)
}

#[tokio::test]
async fn test_anthropic_request_captures_blocks() {
    let (upstream_url, _) = start_mock_upstream().await;
    let (proxy_port, state) = start_test_proxy(&upstream_url, None).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-3-haiku-20240307",
            "max_tokens": 100,
            "messages": [
                { "role": "user", "content": "Hello" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    // Give capture a moment to finalize
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify that blocks were captured
    let blocks = state.capture.all_blocks();
    assert!(!blocks.is_empty(), "Should have captured blocks");

    // Should have a user block from the request
    let user_block = blocks.iter().find(|b| b.role == Role::User);
    assert!(user_block.is_some(), "Should have a user block");
    assert_eq!(user_block.unwrap().content, "Hello");

    // Should have an assistant block from the response
    let assistant_block = blocks.iter().find(|b| b.role == Role::Assistant);
    assert!(assistant_block.is_some(), "Should have an assistant block");
}

#[tokio::test]
async fn test_openai_request_captures_blocks() {
    let (upstream_url, _) = start_mock_upstream().await;
    let (proxy_port, state) = start_test_proxy(&upstream_url, None).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/chat/completions"))
        .header("authorization", "Bearer test-key")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "Hi there" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let blocks = state.capture.all_blocks();
    assert!(!blocks.is_empty());

    // Should have system + user blocks from request
    let system_block = blocks.iter().find(|b| b.role == Role::System);
    assert!(system_block.is_some());
    assert_eq!(system_block.unwrap().content, "You are helpful.");

    let user_block = blocks.iter().find(|b| b.role == Role::User);
    assert!(user_block.is_some());
    assert_eq!(user_block.unwrap().content, "Hi there");
}

#[tokio::test]
async fn test_hot_patch_modifies_forwarded_request() {
    let (upstream_url, _) = start_mock_upstream().await;
    let queue = Arc::new(HotPatchQueue::new());

    // Queue a patch before sending the request
    queue.enqueue(HotPatch {
        role: "user".to_string(),
        original_content: "Hello".to_string(),
        new_content: "Goodbye".to_string(),
        source: PatchSource::Manual,
    });

    let (proxy_port, _state) = start_test_proxy(&upstream_url, Some(queue.clone())).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-3-haiku",
            "max_tokens": 100,
            "messages": [
                { "role": "user", "content": "Hello world" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    // The mock upstream echoes the request body, so patched content should appear
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Goodbye world"),
        "Hot patch should have modified the request. Body: {body}"
    );

    // Queue should be drained after application
    assert!(
        queue.is_empty(),
        "Queue should be empty after applying patches"
    );

    // Captured blocks should reflect the patched content (not pre-patch)
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let blocks = _state.capture.all_blocks();
    let user_block = blocks.iter().find(|b| b.role == Role::User);
    assert!(user_block.is_some(), "Should have captured a user block");
    assert!(
        user_block.unwrap().content.contains("Goodbye world"),
        "Captured block should contain patched content, got: {}",
        user_block.unwrap().content
    );
}

#[tokio::test]
async fn test_hot_patch_no_match_passes_through_unchanged() {
    let (upstream_url, _) = start_mock_upstream().await;
    let queue = Arc::new(HotPatchQueue::new());

    // Queue a patch that won't match anything in the request
    queue.enqueue(HotPatch {
        role: "assistant".to_string(),
        original_content: "nonexistent".to_string(),
        new_content: "replaced".to_string(),
        source: PatchSource::Manual,
    });

    let (proxy_port, _) = start_test_proxy(&upstream_url, Some(queue.clone())).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-3-haiku",
            "max_tokens": 100,
            "messages": [
                { "role": "user", "content": "Hello" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    // Original content should be in the echo
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Hello"),
        "Original content should pass through"
    );

    // Queue should still be drained
    assert!(queue.is_empty());
}

#[tokio::test]
async fn test_response_status_forwarded_correctly() {
    let (upstream_url, _) = start_mock_upstream().await;
    let (proxy_port, _) = start_test_proxy(&upstream_url, None).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-3-haiku",
            "max_tokens": 100,
            "messages": [{ "role": "user", "content": "Test" }]
        }))
        .send()
        .await
        .unwrap();

    // Proxy should forward the 200 status from mock upstream
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_openai_bare_responses_path_forwards_and_captures() {
    let (upstream_url, _) = start_mock_upstream().await;
    let (proxy_port, state) = start_test_proxy(&upstream_url, None).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/responses"))
        .header("authorization", "Bearer session-token-xyz")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Hello from codex" }] }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let blocks = state.capture.all_blocks();
    assert!(
        blocks.iter().any(|b| b.role == Role::Assistant),
        "Expected assistant block from /responses response"
    );
    assert!(
        blocks
            .iter()
            .any(|b| b.content.contains("Hello from codex")),
        "Expected user content captured from /responses request"
    );
}

#[tokio::test]
async fn test_openai_bare_responses_streaming_preserves_query() {
    let (upstream_url, _) = start_mock_upstream().await;
    let (proxy_port, _) = start_test_proxy(&upstream_url, None).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://127.0.0.1:{proxy_port}/responses?trace=1&stream=true"
        ))
        .header("authorization", "Bearer non-sk-token")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "stream": true,
            "require_trace": true,
            "input": "stream test"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let body = response.text().await.unwrap();
    assert!(body.contains("data: [DONE]"));
}

#[tokio::test]
async fn test_proxy_hot_path_overhead_local_average_under_25ms() {
    let (upstream_url, _) = start_mock_upstream().await;
    let (proxy_port, _) = start_test_proxy(&upstream_url, None).await;
    let client = reqwest::Client::new();

    let direct_url = format!("{upstream_url}/v1/messages");
    let proxy_url = format!("http://127.0.0.1:{proxy_port}/v1/messages");
    let payload = serde_json::json!({
        "model": "claude-3-haiku",
        "max_tokens": 64,
        "messages": [{ "role": "user", "content": "latency smoke test" }]
    });

    // Warm up both routes before measuring.
    for _ in 0..3 {
        client
            .post(&direct_url)
            .header("x-api-key", "test-key")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .unwrap();

        client
            .post(&proxy_url)
            .header("x-api-key", "test-key")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .unwrap();
    }

    let samples = 8;
    let mut direct_total_ms = 0f64;
    let mut proxy_total_ms = 0f64;

    for _ in 0..samples {
        let start = Instant::now();
        client
            .post(&direct_url)
            .header("x-api-key", "test-key")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .unwrap();
        direct_total_ms += start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        client
            .post(&proxy_url)
            .header("x-api-key", "test-key")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .unwrap();
        proxy_total_ms += start.elapsed().as_secs_f64() * 1000.0;
    }

    let direct_avg_ms = direct_total_ms / samples as f64;
    let proxy_avg_ms = proxy_total_ms / samples as f64;
    let overhead_ms = (proxy_avg_ms - direct_avg_ms).max(0.0);
    eprintln!(
        "proxy hot-path overhead: direct_avg={direct_avg_ms:.2}ms proxy_avg={proxy_avg_ms:.2}ms overhead={overhead_ms:.2}ms"
    );

    assert!(
        overhead_ms < 25.0,
        "Proxy overhead too high: direct_avg={direct_avg_ms:.2}ms proxy_avg={proxy_avg_ms:.2}ms overhead={overhead_ms:.2}ms"
    );
}
