use super::response::{
    append_assistant_response, build_circuit_breaker_response, is_context_only_response,
    strip_context_calls_from_response,
};
use super::{reinvoke_with_results, MAX_REINVOKE_DEPTH, REINVOKE_TIMEOUT};
use crate::engine::ContextEngine;
use crate::metacog::{self, RuntimeKind};
use crate::proxy::{parser::parse_request, ProxyState};
use axum::http::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

// -- is_context_only_response tests -------------------------------------------------------------

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

// -- strip_context_calls_from_response tests ----------------------------------------------------

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

// -- append_assistant_response tests ------------------------------------------------------------

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

#[test]
fn test_build_circuit_breaker_response_chat_shape() {
    let original = json!({
        "choices": [{
            "message": {"role": "assistant", "content": null}
        }]
    });
    let bytes = build_circuit_breaker_response(RuntimeKind::CodexProxy, &original, 12);
    let json: Value = serde_json::from_slice(&bytes).expect("valid json");
    assert!(json.get("choices").is_some());
    assert!(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .contains("circuit breaker"));
}

#[test]
fn test_build_circuit_breaker_response_responses_shape() {
    let original = json!({
        "output": [{
            "type": "function_call",
            "name": "aperture_context_preview"
        }]
    });
    let bytes = build_circuit_breaker_response(RuntimeKind::CodexProxy, &original, 8);
    let json: Value = serde_json::from_slice(&bytes).expect("valid json");
    assert!(json.get("output").is_some());
    assert_eq!(json["output"][0]["type"], "message");
    assert!(json["output"][0]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("circuit breaker"));
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
