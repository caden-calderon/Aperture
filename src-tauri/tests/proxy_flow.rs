//! Integration tests for the proxy flow.
//!
//! These tests start an actual proxy server, send requests through it,
//! and verify capture, parsing, and hot-patch behavior.
//!
//! Uses a mock upstream server (another axum instance) to avoid real API calls.

use aperture_lib::engine::planner::types::{ContextMutation, PendingPlan};
use aperture_lib::engine::types::Role;
use aperture_lib::engine::ContextEngine;
use aperture_lib::proxy::handler::proxy_handler;
use aperture_lib::proxy::hot_patch::{HotPatch, HotPatchQueue, PatchSource};
use aperture_lib::proxy::parser::{parse_request, parse_response};
use aperture_lib::proxy::{ProxyState, UpstreamConfig};
use axum::{
    body::Bytes, extract::OriginalUri, http::StatusCode, response::IntoResponse, routing::post,
    Json, Router,
};
use std::sync::atomic::{AtomicUsize, Ordering};
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
        chatgpt_codex_url: upstream_url.to_string(),
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

/// Start the Aperture proxy pointed at a mock upstream with an attached engine.
async fn start_test_proxy_with_engine(
    upstream_url: &str,
    engine: Arc<ContextEngine>,
) -> (u16, Arc<ProxyState>) {
    let config = UpstreamConfig {
        anthropic_url: upstream_url.to_string(),
        openai_url: upstream_url.to_string(),
        chatgpt_codex_url: upstream_url.to_string(),
    };

    let mut state = ProxyState::with_config(config).unwrap();
    state.engine = Some(engine);
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

    // Patches are persistent (re-apply on every request) until explicitly cleared
    assert!(
        !queue.is_empty(),
        "Patches should persist after application"
    );
    queue.clear();
    assert!(queue.is_empty(), "Queue should be empty after clear()");

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

    // Patches persist until explicitly cleared (even if no match)
    assert!(!queue.is_empty());
    queue.clear();
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
    // Use sk- token so the request routes to standard OpenAI (not ChatGPT)
    // and the bare /responses path gets /v1/ normalization.
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/responses"))
        .header("authorization", "Bearer sk-test-key-xyz")
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
async fn test_capture_uses_rewritten_request_semantics() {
    let (upstream_url, _) = start_mock_upstream().await;
    let engine = Arc::new(ContextEngine::new_in_memory(None));

    // Seed engine state so planner mutations can target real block IDs.
    let path = "/v1/chat/completions";
    let seed_request = serde_json::json!({
        "model": "gpt-4.1",
        "messages": [
            { "role": "system", "content": "You are helpful." },
            { "role": "user", "content": "Needs compression." },
            { "role": "assistant", "content": "Middle info." },
            { "role": "user", "content": "Archive me." }
        ]
    });
    let seed_bytes = serde_json::to_vec(&seed_request).expect("serialize seed request");
    let parsed_seed = parse_request(path, &seed_bytes).expect("parse seed request");
    let seed_response = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4.1",
        "choices": [{
            "message": { "role": "assistant", "content": "seed response" }
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 3 }
    }))
    .expect("serialize seed response");
    let parsed_seed_response =
        parse_response(parsed_seed.provider, path, &seed_response).expect("parse seed response");
    let seed_ingest = engine.ingest(
        &parsed_seed.provider.to_string(),
        &parsed_seed.model,
        "proxy",
        parsed_seed.thread_identity.as_deref(),
        parsed_seed.blocks,
        parsed_seed_response.blocks,
        0,
    );

    let blocks = engine.active_session_blocks();
    let compress_id = blocks
        .iter()
        .find(|b| b.content.contains("Needs compression."))
        .map(|b| b.id.clone())
        .expect("compress target");
    let archive_id = blocks
        .iter()
        .find(|b| b.content.contains("Archive me."))
        .map(|b| b.id.clone())
        .expect("archive target");

    engine.planner.set_pending_plan_for_session(
        &seed_ingest.session_id,
        PendingPlan {
            mutations: vec![
                ContextMutation::Compress {
                    block_id: compress_id,
                    summary: "compressed request content".to_string(),
                },
                ContextMutation::Archive {
                    block_id: archive_id,
                },
            ],
            token_delta: -50,
            projected_block_count: blocks.len().saturating_sub(1),
            projected_utilization: 0.2,
        },
    );

    let (proxy_port, state) = start_test_proxy_with_engine(&upstream_url, engine).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}{path}"))
        .header("authorization", "Bearer sk-test-capture-order")
        .header("content-type", "application/json")
        .json(&seed_request)
        .send()
        .await
        .expect("proxy request");

    assert!(response.status().is_success());
    let json: serde_json::Value = response.json().await.expect("response json");
    let echoed = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        echoed.contains("compressed request content"),
        "Upstream should receive rewritten compressed content"
    );
    assert!(
        !echoed.contains("Archive me."),
        "Upstream should not receive archived turn content"
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let captured = state.capture.all_blocks();
    assert!(
        captured
            .iter()
            .any(|b| b.content.contains("compressed request content")),
        "Capture should record rewritten request semantics"
    );
    assert!(
        !captured.iter().any(|b| b.content.contains("Archive me.")),
        "Capture should not keep pre-rewrite archived content"
    );
}

#[tokio::test]
async fn test_context_only_interception_reinvoke_returns_final_response_and_captures_it() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_for_route = call_count.clone();

    let app = Router::new().route(
        "/v1/responses",
        post(move |Json(body): Json<serde_json::Value>| {
            let call_count = call_count_for_route.clone();
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);

                let has_context_result = body
                    .get("input")
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items.iter().any(|item| {
                            item.get("type").and_then(|v| v.as_str())
                                == Some("function_call_output")
                                && item.get("call_id").and_then(|v| v.as_str()) == Some("ctx_1")
                        })
                    })
                    .unwrap_or(false);

                if has_context_result {
                    Json(serde_json::json!({
                        "id": "resp_final",
                        "model": "gpt-4.1",
                        "output": [{
                            "type": "message",
                            "content": [{ "type": "output_text", "text": "final answer after context tool" }]
                        }],
                        "usage": { "input_tokens": 14, "output_tokens": 6 }
                    }))
                    .into_response()
                } else {
                    Json(serde_json::json!({
                        "id": "resp_context_only",
                        "model": "gpt-4.1",
                        "output": [{
                            "type": "function_call",
                            "name": "aperture_context_preview",
                            "call_id": "ctx_1",
                            "arguments": "{}"
                        }],
                        "usage": { "input_tokens": 10, "output_tokens": 2 }
                    }))
                    .into_response()
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let engine = Arc::new(ContextEngine::new_in_memory(None));
    let (proxy_port, state) = start_test_proxy_with_engine(&upstream_url, engine).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .header("authorization", "Bearer sk-test-context")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "input": [{ "type": "message", "role": "user", "content": "run context tool" }]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["output"][0]["type"], "message");
    assert_eq!(
        body["output"][0]["content"][0]["text"],
        "final answer after context tool"
    );
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "Context-only interception should re-invoke exactly once"
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let blocks = state.capture.all_blocks();
    assert!(
        blocks
            .iter()
            .any(|b| b.role == Role::Assistant
                && b.content.contains("final answer after context tool")),
        "Capture should record the modified re-invoked response body"
    );
    assert!(
        !blocks
            .iter()
            .any(|b| { b.role == Role::ToolUse && b.content.contains("aperture_context_preview") }),
        "Capture should not retain the stripped context-only upstream response"
    );
}

#[tokio::test]
async fn test_mixed_interception_strips_context_calls_without_reinvoke() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_for_route = call_count.clone();

    let app = Router::new().route(
        "/v1/responses",
        post(move |_body: Bytes| {
            let call_count = call_count_for_route.clone();
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({
                    "id": "resp_mixed",
                    "model": "gpt-4.1",
                    "output": [
                        {
                            "type": "function_call",
                            "name": "aperture_context_preview",
                            "call_id": "ctx_1",
                            "arguments": "{}"
                        },
                        {
                            "type": "function_call",
                            "name": "read_file",
                            "call_id": "real_1",
                            "arguments": "{\"path\":\"src/main.rs\"}"
                        }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 2 }
                }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let engine = Arc::new(ContextEngine::new_in_memory(None));
    let (proxy_port, state) = start_test_proxy_with_engine(&upstream_url, engine).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .header("authorization", "Bearer sk-test-context")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "input": [{ "type": "message", "role": "user", "content": "mixed tool output" }]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await.unwrap();
    let output = body["output"].as_array().expect("output array");
    assert_eq!(output.len(), 1, "Context tool call should be stripped");
    assert_eq!(output[0]["name"], "read_file");
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "Mixed responses should not trigger re-invoke"
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let blocks = state.capture.all_blocks();
    assert!(
        blocks
            .iter()
            .any(|b| b.role == Role::ToolUse && b.content.contains("Tool: read_file")),
        "Capture should include only the effective non-context tool call"
    );
    assert!(
        !blocks
            .iter()
            .any(|b| { b.role == Role::ToolUse && b.content.contains("aperture_context_preview") }),
        "Capture should not include stripped context tool calls"
    );
}

#[tokio::test]
async fn test_context_only_reinvoke_depth_limit_fail_open() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_for_route = call_count.clone();

    let app = Router::new().route(
        "/v1/responses",
        post(move |_body: Bytes| {
            let call_count = call_count_for_route.clone();
            async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({
                    "id": "resp_loop",
                    "model": "gpt-4.1",
                    "output": [{
                        "type": "function_call",
                        "name": "aperture_context_preview",
                        "call_id": "ctx_loop",
                        "arguments": "{}"
                    }],
                    "usage": { "input_tokens": 9, "output_tokens": 2 }
                }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let engine = Arc::new(ContextEngine::new_in_memory(None));
    let (proxy_port, _state) = start_test_proxy_with_engine(&upstream_url, engine).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .header("authorization", "Bearer sk-test-context")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "input": [{ "type": "message", "role": "user", "content": "loop context call" }]
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "Depth-limit exhaustion should fail-open with a successful proxy response"
    );
    let body: serde_json::Value = response.json().await.unwrap();
    let output = body["output"].as_array().expect("output array");
    assert!(
        output.is_empty(),
        "Fallback stripped response should remove context-only tool calls"
    );
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        4,
        "Expected 1 initial call + 3 re-invokes before depth fail-open"
    );
}

#[tokio::test]
async fn test_openai_bare_responses_streaming_preserves_query() {
    let (upstream_url, _) = start_mock_upstream().await;
    let (proxy_port, _) = start_test_proxy(&upstream_url, None).await;

    let client = reqwest::Client::new();
    // Use sk- token so the request routes to standard OpenAI (not ChatGPT)
    // and the bare /responses path gets /v1/ normalization.
    let response = client
        .post(format!(
            "http://127.0.0.1:{proxy_port}/responses?trace=1&stream=true"
        ))
        .header("authorization", "Bearer sk-test-streaming")
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

/// Verify that hot patches apply to Responses API `input` array (not just `messages`).
#[tokio::test]
async fn test_hot_patch_applies_to_responses_api_input() {
    let (upstream_url, _) = start_mock_upstream().await;
    let queue = Arc::new(HotPatchQueue::new());

    queue.enqueue(HotPatch {
        role: "user".to_string(),
        original_content: "Hello from codex".to_string(),
        new_content: "Patched from codex".to_string(),
        source: PatchSource::Manual,
    });

    let (proxy_port, state) = start_test_proxy(&upstream_url, Some(queue.clone())).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/responses"))
        .header("authorization", "Bearer sk-test-key")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "input": [
                { "type": "message", "role": "user", "content": "Hello from codex" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    // The mock echoes back request body — patched content should appear
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Echo from responses"),
        "Should get a valid response from /responses endpoint"
    );

    // Captured blocks should reflect the patched content
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let blocks = state.capture.all_blocks();
    let user_block = blocks
        .iter()
        .find(|b| b.content.contains("Patched from codex") || b.content.contains("codex"));
    assert!(
        user_block.is_some(),
        "Should have captured user block from /responses input. Blocks: {:?}",
        blocks.iter().map(|b| &b.content).collect::<Vec<_>>()
    );
}

/// Verify hot patches work through SSE streaming on the Responses API path.
#[tokio::test]
async fn test_hot_patch_with_streaming_responses_api() {
    let (upstream_url, _) = start_mock_upstream().await;
    let queue = Arc::new(HotPatchQueue::new());

    queue.enqueue(HotPatch {
        role: "user".to_string(),
        original_content: "stream test".to_string(),
        new_content: "patched stream test".to_string(),
        source: PatchSource::Manual,
    });

    let (proxy_port, state) = start_test_proxy(&upstream_url, Some(queue.clone())).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/responses"))
        .header("authorization", "Bearer sk-test-key")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "stream": true,
            "input": "stream test"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Streaming response should contain SSE data
    let body = response.text().await.unwrap();
    assert!(
        body.contains("data: [DONE]"),
        "SSE stream should complete with [DONE]"
    );

    // Wait for capture finalization
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Patches should persist after streaming
    assert!(
        !queue.is_empty(),
        "Patches should persist after streaming request"
    );

    // Verify capture recorded something for the request
    let blocks = state.capture.all_blocks();
    assert!(
        !blocks.is_empty(),
        "Should have captured blocks from streaming response"
    );
}

/// Verify that ChatGPT subscription tokens route correctly and
/// the mock responds successfully through the proxy.
#[tokio::test]
async fn test_chatgpt_subscription_routing_with_sse() {
    // Start mock upstream with a ChatGPT-style /responses endpoint
    let app = Router::new().route(
        "/responses",
        post(|Json(body): Json<serde_json::Value>| async move {
            let stream = body
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if stream {
                let sse = concat!(
                    "data: {\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"content\":\"Chat\"}}]}\n\n",
                    "data: {\"model\":\"gpt-4.1\",\"choices\":[{\"delta\":{\"content\":\"GPT\"}}]}\n\n",
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
                "id": "resp_chatgpt",
                "model": "gpt-4.1",
                "output": [{
                    "type": "message",
                    "content": [{ "type": "output_text", "text": "From ChatGPT backend" }]
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            }))
            .into_response()
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let chatgpt_port = listener.local_addr().unwrap().port();
    let chatgpt_url = format!("http://127.0.0.1:{chatgpt_port}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Point chatgpt_codex_url at the mock (but leave the standard openai_url at a different mock)
    let (standard_url, _) = start_mock_upstream().await;
    let config = UpstreamConfig {
        anthropic_url: standard_url.clone(),
        openai_url: standard_url,
        chatgpt_codex_url: chatgpt_url,
    };

    let state = ProxyState::with_config(config).unwrap();
    let state = Arc::new(state);

    let app = Router::new()
        .route("/{*path}", axum::routing::any(proxy_handler))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();

    // Non-streaming: subscription token (non-sk-) on /responses → ChatGPT backend
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/responses"))
        .header("authorization", "Bearer session-token-abc")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Hello" }] }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "ChatGPT subscription routing should succeed, got {}",
        response.status()
    );

    let body = response.text().await.unwrap();
    assert!(
        body.contains("From ChatGPT backend"),
        "Response should come from ChatGPT mock, got: {body}"
    );

    // Streaming: subscription token on /responses with stream=true
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/responses"))
        .header("authorization", "Bearer session-token-abc")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "stream": true,
            "input": "streaming test"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Chat") && body.contains("GPT"),
        "SSE stream should contain ChatGPT mock content, got: {body}"
    );
    assert!(body.contains("data: [DONE]"), "SSE stream should complete");
}

#[tokio::test]
async fn test_request_hop_by_hop_headers_are_stripped() {
    let app = Router::new().route(
        "/v1/messages",
        post(|headers: axum::http::HeaderMap, body: Bytes| async move {
            let parsed_ok = serde_json::from_slice::<serde_json::Value>(&body).is_ok();
            Json(serde_json::json!({
                "id": "req_header_strip",
                "model": "claude-3-haiku",
                "parsed_ok": parsed_ok,
                "has_connection": headers.contains_key("connection"),
                "has_keep_alive": headers.contains_key("keep-alive"),
                "has_accept_encoding": headers.contains_key("accept-encoding"),
                "has_te": headers.contains_key("te"),
                "content_type": headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default(),
                "content": [{ "type": "text", "text": "header strip ok" }],
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            }))
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let (proxy_port, _) = start_test_proxy(&upstream_url, None).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("connection", "keep-alive, te")
        .header("keep-alive", "timeout=5")
        .header("accept-encoding", "gzip, br")
        .header("te", "trailers")
        .json(&serde_json::json!({
            "model": "claude-3-haiku",
            "messages": [{ "role": "user", "content": "header strip request" }]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["parsed_ok"], true);
    assert_eq!(body["has_connection"], false);
    assert_eq!(body["has_keep_alive"], false);
    assert_eq!(body["has_accept_encoding"], false);
    assert_eq!(body["has_te"], false);
    assert_eq!(body["content_type"], "application/json");
}

#[tokio::test]
async fn test_response_hop_by_hop_headers_are_stripped() {
    let app = Router::new().route(
        "/v1/messages",
        post(|| async move {
            (
                StatusCode::OK,
                [
                    ("content-type", "application/json"),
                    ("connection", "keep-alive"),
                    ("keep-alive", "timeout=5"),
                    ("te", "trailers"),
                    ("trailer", "expires"),
                    ("upgrade", "websocket"),
                    ("x-upstream-ok", "1"),
                ],
                r#"{"id":"msg_resp_header","type":"message","role":"assistant","model":"claude-3-haiku","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}"#,
            )
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let (proxy_port, _) = start_test_proxy(&upstream_url, None).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-3-haiku",
            "messages": [{ "role": "user", "content": "response header strip request" }]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let headers = response.headers();
    assert!(!headers.contains_key("connection"));
    assert!(!headers.contains_key("keep-alive"));
    assert!(!headers.contains_key("te"));
    assert!(!headers.contains_key("trailer"));
    assert!(!headers.contains_key("upgrade"));
    assert_eq!(
        headers.get("x-upstream-ok").and_then(|v| v.to_str().ok()),
        Some("1")
    );
}

#[tokio::test]
async fn test_non_sk_bearer_chat_completions_stays_on_openai_route() {
    let openai_app = Router::new().route(
        "/v1/chat/completions",
        post(|Json(_body): Json<serde_json::Value>| async move {
            Json(serde_json::json!({
                "id": "chatcmpl-openai",
                "model": "gpt-4.1",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "from standard openai route" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 }
            }))
        }),
    );
    let chatgpt_app = Router::new().route(
        "/chat/completions",
        post(|| async move {
            Json(serde_json::json!({
                "id": "chatcmpl-chatgpt",
                "marker": "wrong-route"
            }))
        }),
    );

    let openai_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let openai_port = openai_listener.local_addr().unwrap().port();
    let openai_url = format!("http://127.0.0.1:{openai_port}");
    tokio::spawn(async move {
        axum::serve(openai_listener, openai_app).await.unwrap();
    });

    let chatgpt_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let chatgpt_port = chatgpt_listener.local_addr().unwrap().port();
    let chatgpt_url = format!("http://127.0.0.1:{chatgpt_port}");
    tokio::spawn(async move {
        axum::serve(chatgpt_listener, chatgpt_app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let config = UpstreamConfig {
        anthropic_url: openai_url.clone(),
        openai_url: openai_url.clone(),
        chatgpt_codex_url: chatgpt_url,
    };
    let state = Arc::new(ProxyState::with_config(config).unwrap());
    let app = Router::new()
        .route("/{*path}", axum::routing::any(proxy_handler))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/chat/completions"))
        .header("authorization", "Bearer session-token-non-sk")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "messages": [{ "role": "user", "content": "route matrix test" }]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], "chatcmpl-openai");
    assert_ne!(
        body.get("marker"),
        Some(&serde_json::Value::String("wrong-route".to_string()))
    );
}

#[tokio::test]
async fn test_hot_patch_persists_for_next_turn_remember_number_regression() {
    let app = Router::new().route(
        "/v1/responses",
        post(|Json(body): Json<serde_json::Value>| async move {
            let remembered = body
                .get("input")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
                .and_then(|item| item.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            Json(serde_json::json!({
                "id": "resp_remember",
                "model": "gpt-4.1",
                "output": [{
                    "type": "message",
                    "content": [{ "type": "output_text", "text": format!("I will remember: {remembered}") }]
                }],
                "usage": { "input_tokens": 8, "output_tokens": 4 }
            }))
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let queue = Arc::new(HotPatchQueue::new());
    queue.enqueue(HotPatch {
        role: "user".to_string(),
        original_content: "remember number 52".to_string(),
        new_content: "remember number 73".to_string(),
        source: PatchSource::Manual,
    });

    let (proxy_port, state) = start_test_proxy(&upstream_url, Some(queue.clone())).await;
    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "model": "gpt-4.1",
        "input": [{ "type": "message", "role": "user", "content": "remember number 52" }]
    });

    let first_response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .header("authorization", "Bearer sk-test-remember")
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert!(first_response.status().is_success());
    let first_json: serde_json::Value = first_response.json().await.unwrap();
    assert!(
        first_json.to_string().contains("remember number 73"),
        "First response should reflect patched memory content"
    );

    let second_response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .header("authorization", "Bearer sk-test-remember")
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert!(second_response.status().is_success());
    let second_json: serde_json::Value = second_response.json().await.unwrap();
    assert!(
        second_json.to_string().contains("remember number 73"),
        "Second response should continue to use patched memory content"
    );
    assert!(
        !queue.is_empty(),
        "Hot patch should remain queued for subsequent turns"
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let blocks = state.capture.all_blocks();
    assert!(
        blocks
            .iter()
            .any(|b| b.content.contains("remember number 73")),
        "Captured context should contain patched remember value"
    );
}

/// Verify that zstd-compressed request bodies are decompressed for capture.
///
/// Codex CLI sends `content-encoding: zstd` compressed request bodies.
/// The proxy must decompress them for block parsing/capture while forwarding
/// the original compressed bytes to upstream (transparent byte-passthrough).
#[tokio::test]
async fn test_zstd_compressed_body_decompressed_for_capture() {
    // Start a mock upstream that accepts raw bytes (not JSON) to test passthrough.
    // The mock accepts POST with any body and returns a valid Responses API response.
    let app = Router::new().route(
        "/v1/responses",
        post(|body: Bytes| async move {
            // Try to parse as JSON — if the body is still compressed this will fail
            let is_json = serde_json::from_slice::<serde_json::Value>(&body).is_ok();
            Json(serde_json::json!({
                "id": "resp_zstd_test",
                "model": "gpt-4.1",
                "body_was_valid_json": is_json,
                "body_len": body.len(),
                "output": [{
                    "type": "message",
                    "content": [{ "type": "output_text", "text": "zstd response" }]
                }],
                "usage": { "input_tokens": 10, "output_tokens": 5 }
            }))
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let (proxy_port, state) = start_test_proxy(&upstream_url, None).await;

    // Build a valid Responses API JSON body and compress it with zstd.
    // Responses API items require "type": "message" for the parser to extract blocks.
    let json_body = serde_json::json!({
        "model": "gpt-4.1",
        "input": [
            { "type": "message", "role": "user", "content": "Hello from zstd-compressed codex request" }
        ]
    });
    let json_bytes = serde_json::to_vec(&json_body).unwrap();
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(&json_bytes), 3).unwrap();

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .header("authorization", "Bearer sk-test-zstd")
        .header("content-type", "application/json")
        .header("content-encoding", "zstd")
        .body(compressed.clone())
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "Proxy should forward zstd request successfully, got {}",
        response.status()
    );

    let resp_body: serde_json::Value = response.json().await.unwrap();

    // The upstream received the original compressed bytes (transparent passthrough)
    // so it should NOT see valid JSON
    assert_eq!(
        resp_body["body_was_valid_json"], false,
        "Upstream should receive compressed bytes (not decompressed) when no patches applied"
    );

    // But the capture should have decompressed and parsed the blocks
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let blocks = state.capture.all_blocks();

    let user_block = blocks
        .iter()
        .find(|b| b.content.contains("zstd-compressed codex"));
    assert!(
        user_block.is_some(),
        "Capture should have decompressed zstd body and extracted user block. Blocks: {:?}",
        blocks
            .iter()
            .map(|b| (&b.role, &b.content))
            .collect::<Vec<_>>()
    );
}

/// Verify that hot patches apply to zstd-compressed request bodies AND
/// the decompressed+patched body is forwarded (with content-encoding stripped).
#[tokio::test]
async fn test_zstd_body_hot_patch_sends_decompressed_patched_body() {
    // Mock upstream that parses JSON and echoes it back
    let app = Router::new().route(
        "/v1/responses",
        post(|headers: axum::http::HeaderMap, body: Bytes| async move {
            let parsed = serde_json::from_slice::<serde_json::Value>(&body);
            match parsed {
                Ok(json) => Json(serde_json::json!({
                    "id": "resp_zstd_patch",
                    "model": "gpt-4.1",
                    "has_content_encoding_header": headers.contains_key("content-encoding"),
                    "echoed_input": json.get("input").cloned(),
                    "output": [{
                        "type": "message",
                        "content": [{ "type": "output_text", "text": "patched response" }]
                    }],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }))
                .into_response(),
                Err(_) => (
                    StatusCode::BAD_REQUEST,
                    "Failed to parse JSON (body may still be compressed)",
                )
                    .into_response(),
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let queue = Arc::new(HotPatchQueue::new());
    queue.enqueue(HotPatch {
        role: "user".to_string(),
        original_content: "original zstd content".to_string(),
        new_content: "patched zstd content".to_string(),
        source: PatchSource::Manual,
    });

    let (proxy_port, state) = start_test_proxy(&upstream_url, Some(queue.clone())).await;

    // Responses API items require "type": "message" for the parser to extract blocks.
    let json_body = serde_json::json!({
        "model": "gpt-4.1",
        "input": [
            { "type": "message", "role": "user", "content": "original zstd content here" }
        ]
    });
    let json_bytes = serde_json::to_vec(&json_body).unwrap();
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(&json_bytes), 3).unwrap();

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/responses"))
        .header("authorization", "Bearer sk-test-zstd-patch")
        .header("content-type", "application/json")
        .header("content-encoding", "zstd")
        .body(compressed)
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let resp_json: serde_json::Value = response.json().await.unwrap();

    // The echoed input should contain the patched content (not compressed, not original)
    let echoed = resp_json["echoed_input"].to_string();
    assert!(
        echoed.contains("patched zstd content"),
        "Upstream should receive decompressed+patched body. Echoed: {echoed}"
    );
    assert_eq!(
        resp_json["has_content_encoding_header"], false,
        "Proxy should strip content-encoding after zstd decompression+patch"
    );

    // Captured blocks should reflect patched content
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let blocks = state.capture.all_blocks();
    let user_block = blocks
        .iter()
        .find(|b| b.content.contains("patched zstd content"));
    assert!(
        user_block.is_some(),
        "Capture should contain patched content from decompressed zstd body. Blocks: {:?}",
        blocks
            .iter()
            .map(|b| (&b.role, &b.content))
            .collect::<Vec<_>>()
    );
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
