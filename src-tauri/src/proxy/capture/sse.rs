use super::Provider;
use crate::proxy::parser;

/// Extract the final complete response JSON from an SSE stream buffer.
///
/// For Anthropic: looks for `event: message_stop` then finds the last
/// `event: message_delta` with usage data, or reconstructs from content deltas.
///
/// For OpenAI: looks for `data: [DONE]` marker and accumulates content deltas.
pub(super) fn extract_final_response(
    sse_buffer: &str,
    provider: Provider,
    path: &str,
) -> Option<Vec<u8>> {
    match provider {
        Provider::Anthropic => extract_anthropic_final_response(sse_buffer),
        Provider::OpenAI => {
            if parser::is_responses_path(path) {
                extract_openai_responses_final_response(sse_buffer)
            } else {
                extract_openai_chat_final_response(sse_buffer)
            }
        }
    }
}

/// Reconstruct an Anthropic response from SSE events.
///
/// Anthropic SSE events for streaming:
/// - `message_start` with `message: {id, model, role, usage: {input_tokens}}`
/// - `content_block_start` with `content_block: {type: "text", text: ""}`
/// - `content_block_delta` with `delta: {type: "text_delta", text: "chunk"}`
/// - `content_block_stop`
/// - `message_delta` with `delta: {stop_reason}, usage: {output_tokens}`
/// - `message_stop`
pub(super) fn extract_anthropic_final_response(sse_buffer: &str) -> Option<Vec<u8>> {
    let mut model = None;
    let mut accumulated_text = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    for line in sse_buffer.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..]; // Skip "data: "
        if data == "[DONE]" {
            break;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match event_type {
                "message_start" => {
                    if let Some(msg) = json.get("message") {
                        model = msg
                            .get("model")
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string());
                        if let Some(usage) = msg.get("usage") {
                            input_tokens = usage
                                .get("input_tokens")
                                .and_then(|t| t.as_u64())
                                .unwrap_or(0) as u32;
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = json.get("delta") {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                            accumulated_text.push_str(text);
                        }
                    }
                }
                "message_delta" => {
                    if let Some(usage) = json.get("usage") {
                        output_tokens = usage
                            .get("output_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0) as u32;
                    }
                }
                _ => {}
            }
        }
    }

    if accumulated_text.is_empty() && input_tokens == 0 {
        return None;
    }

    // Reconstruct a complete response JSON.
    let response = serde_json::json!({
        "model": model,
        "content": [{"type": "text", "text": accumulated_text}],
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    });

    Some(serde_json::to_vec(&response).unwrap_or_default())
}

/// Reconstruct an OpenAI response from SSE events.
///
/// OpenAI SSE events for streaming:
/// - `data: {choices: [{delta: {role: "assistant"}}]}`
/// - `data: {choices: [{delta: {content: "chunk"}}]}`
/// - `data: {choices: [{delta: {}, finish_reason: "stop"}], usage: {...}}`
/// - `data: [DONE]`
pub(super) fn extract_openai_chat_final_response(sse_buffer: &str) -> Option<Vec<u8>> {
    let mut model = None;
    let mut accumulated_content = String::new();
    let mut prompt_tokens: u32 = 0;
    let mut completion_tokens: u32 = 0;

    for line in sse_buffer.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            if model.is_none() {
                model = json
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
            }

            if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                            accumulated_content.push_str(content);
                        }
                    }
                }
            }

            if let Some(usage) = json.get("usage") {
                prompt_tokens = usage
                    .get("prompt_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
                completion_tokens = usage
                    .get("completion_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
            }
        }
    }

    if accumulated_content.is_empty() && prompt_tokens == 0 {
        return None;
    }

    let response = serde_json::json!({
        "model": model,
        "choices": [{
            "message": {"role": "assistant", "content": accumulated_content}
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    });

    Some(serde_json::to_vec(&response).unwrap_or_default())
}

/// Reconstruct an OpenAI Responses API response from SSE events.
///
/// Handles common event patterns:
/// - `response.output_text.delta` with `delta: "chunk"`
/// - final events that include `usage`
/// - optional full `output` payload in completion events
pub(super) fn extract_openai_responses_final_response(sse_buffer: &str) -> Option<Vec<u8>> {
    let mut model = None;
    let mut accumulated_text = String::new();
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

    for line in sse_buffer.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            if model.is_none() {
                model = json
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
            }

            // Streaming delta events.
            if json.get("type").and_then(|t| t.as_str()) == Some("response.output_text.delta") {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    accumulated_text.push_str(delta);
                }
            }

            // Some providers include full output arrays in terminal events.
            if accumulated_text.is_empty() {
                if let Some(output) = json.get("output").and_then(|o| o.as_array()) {
                    for item in output {
                        if item.get("type").and_then(|t| t.as_str()) == Some("message") {
                            if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                                for part in content {
                                    let part_type =
                                        part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if (part_type == "output_text" || part_type == "text")
                                        && part.get("text").and_then(|t| t.as_str()).is_some()
                                    {
                                        accumulated_text.push_str(
                                            part.get("text")
                                                .and_then(|t| t.as_str())
                                                .unwrap_or_default(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(usage) = json.get("usage") {
                input_tokens = usage
                    .get("input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
                output_tokens = usage
                    .get("output_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as u32;
            }
        }
    }

    if accumulated_text.is_empty() && input_tokens == 0 {
        return None;
    }

    let response = serde_json::json!({
        "model": model,
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": accumulated_text
            }]
        }],
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    });

    Some(serde_json::to_vec(&response).unwrap_or_default())
}
