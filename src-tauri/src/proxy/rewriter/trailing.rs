use serde_json::Value;

use crate::proxy::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

/// Append context text (warnings/breadcrumbs) to the last user message
/// instead of modifying the system prompt.
pub(super) fn inject_trailing_context(json: &mut Value, path: &str, text: &str) {
    if is_messages_path(path) {
        inject_trailing_anthropic(json, text);
    } else if is_chat_completions_path(path) {
        inject_trailing_openai_chat(json, text);
    } else if is_responses_path(path) {
        inject_trailing_openai_responses(json, text);
    }
}

/// Anthropic: append to last user message in `messages[]`.
fn inject_trailing_anthropic(json: &mut Value, text: &str) {
    let messages = match json.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };

    let last_user_idx = messages
        .iter()
        .rposition(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"));

    let Some(idx) = last_user_idx else {
        return;
    };

    let msg = &mut messages[idx];
    append_to_anthropic_content(msg, text);
}

/// Append text to an Anthropic message's content (string or array form).
fn append_to_anthropic_content(msg: &mut Value, text: &str) {
    match msg.get("content") {
        Some(Value::String(existing)) => {
            let new_content = format!("{existing}\n\n{text}");
            msg["content"] = Value::String(new_content);
        }
        Some(Value::Array(_)) => {
            if let Some(arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                arr.push(serde_json::json!({
                    "type": "text",
                    "text": format!("\n{text}")
                }));
            }
        }
        _ => {
            msg["content"] = Value::String(text.to_string());
        }
    }
}

/// OpenAI Chat Completions: append to last user message in `messages[]`.
fn inject_trailing_openai_chat(json: &mut Value, text: &str) {
    let messages = match json.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };

    let last_user_idx = messages
        .iter()
        .rposition(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"));

    let Some(idx) = last_user_idx else {
        return;
    };

    let msg = &mut messages[idx];
    append_to_openai_item_content(msg, text, "text");
}

/// OpenAI Responses: append to last user input item in `input[]`.
fn inject_trailing_openai_responses(json: &mut Value, text: &str) {
    match json.get_mut("input") {
        Some(Value::String(existing)) => {
            let new_content = format!("{existing}\n\n{text}");
            json["input"] = Value::String(new_content);
        }
        Some(Value::Object(item)) => {
            let mut value = Value::Object(item.clone());
            append_to_openai_item_content(&mut value, text, "input_text");
            json["input"] = value;
        }
        Some(Value::Array(input)) => {
            let last_user_idx = input
                .iter()
                .rposition(|item| item.get("role").and_then(|r| r.as_str()) == Some("user"))
                .or_else(|| {
                    input.iter().rposition(|item| {
                        item.get("type").and_then(|t| t.as_str()) == Some("message")
                    })
                });

            let Some(idx) = last_user_idx else {
                return;
            };

            let item = &mut input[idx];
            append_to_openai_item_content(item, text, "input_text");
        }
        _ => {}
    }
}

fn append_to_openai_item_content(item: &mut Value, text: &str, text_part_type: &str) {
    let Some(obj) = item.as_object_mut() else {
        return;
    };

    match obj.get_mut("content") {
        Some(Value::String(existing)) => {
            let new_content = format!("{existing}\n\n{text}");
            *existing = new_content;
        }
        Some(Value::Array(arr)) => {
            arr.push(serde_json::json!({
                "type": text_part_type,
                "text": format!("\n{text}")
            }));
        }
        Some(Value::Null) | None => {
            obj.insert("content".to_string(), Value::String(text.to_string()));
        }
        _ => {}
    }
}
