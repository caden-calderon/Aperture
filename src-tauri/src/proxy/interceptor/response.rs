use serde_json::Value;

use crate::metacog::{is_context_tool_name, RuntimeKind};

pub(super) fn build_circuit_breaker_response(
    runtime_kind: RuntimeKind,
    original_response: &Value,
    wait_secs: u64,
) -> Vec<u8> {
    let wait_secs = wait_secs.max(1);

    let body = if runtime_kind == RuntimeKind::CodexProxy
        && original_response.get("output").is_some()
    {
        serde_json::json!({
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": format!(
                        "Aperture circuit breaker active: context tools are temporarily blocked after runaway usage. Retry in ~{}s.",
                        wait_secs
                    )
                }]
            }]
        })
    } else {
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": format!(
                        "Aperture circuit breaker active: context tools are temporarily blocked after runaway usage. Retry in ~{}s.",
                        wait_secs
                    )
                },
                "finish_reason": "stop"
            }]
        })
    };

    serde_json::to_vec(&body).unwrap_or_else(|_| b"{\"error\":\"circuit_breaker\"}".to_vec())
}

/// Check if a response contains only context tool calls (no text, no real tools).
pub(super) fn is_context_only_response(response_json: &Value, runtime_kind: RuntimeKind) -> bool {
    match runtime_kind {
        RuntimeKind::CodexProxy => {
            // Check if the path indicates Responses or ChatCompletions format
            // by looking at response structure.
            if let Some(output) = response_json.get("output").and_then(|o| o.as_array()) {
                // Responses API format.
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
                // ChatCompletions format.
                choices.iter().all(|choice| {
                    let msg = match choice.get("message") {
                        Some(m) => m,
                        None => return false,
                    };
                    // No text content.
                    let has_text = msg
                        .get("content")
                        .and_then(|c| c.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false);
                    if has_text {
                        return false;
                    }
                    // All tool calls are context tools.
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
pub(super) fn strip_context_calls_from_response(
    mut response_json: Value,
    runtime_kind: RuntimeKind,
) -> Value {
    if runtime_kind == RuntimeKind::CodexProxy {
        if let Some(output) = response_json
            .get_mut("output")
            .and_then(|o| o.as_array_mut())
        {
            // Responses API: remove function_call items that are context tools.
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
            // ChatCompletions: remove context tool_calls from each choice.
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
                        // If no tool_calls remain, remove the key.
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

/// Append the assistant's response to the request body for re-invocation.
pub(super) fn append_assistant_response(
    request_json: &mut Value,
    response_json: &Value,
    runtime_kind: RuntimeKind,
) {
    if runtime_kind != RuntimeKind::CodexProxy {
        return;
    }

    if let Some(output) = response_json.get("output").and_then(|o| o.as_array()) {
        // Responses API: append output items to input[].
        if let Some(input) = request_json.get_mut("input").and_then(|i| i.as_array_mut()) {
            for item in output {
                input.push(item.clone());
            }
        }
    } else if let Some(choices) = response_json.get("choices").and_then(|c| c.as_array()) {
        // ChatCompletions: append assistant message to messages[].
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
