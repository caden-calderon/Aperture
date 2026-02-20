use std::collections::HashSet;

use serde_json::Value;
use tracing::warn;

use crate::engine::planner::applicator::{PartialTurnStub, RewriteDecisions};
use crate::proxy::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

/// Apply rewrite decisions to a JSON value, dispatching to the correct format.
pub(super) fn apply_decisions_to_json(json: &mut Value, path: &str, decisions: &RewriteDecisions) {
    // Order: stubs → replacements → removal.
    // Stubs and replacements modify content WITHIN messages using original indices.
    // Removal deletes entire messages and shifts indices — must happen last.
    if is_messages_path(path) {
        apply_partial_turn_stubs_anthropic(json, &decisions.partial_turn_stubs);
        for (turn, content) in &decisions.content_replacements {
            replace_anthropic_content(json, *turn, content);
        }
        remove_anthropic_messages(json, &decisions.remove_turns);
    } else if is_chat_completions_path(path) {
        apply_partial_turn_stubs_openai_chat(json, &decisions.partial_turn_stubs);
        for (turn, content) in &decisions.content_replacements {
            replace_openai_chat_content(json, *turn, content);
        }
        remove_openai_chat_messages(json, &decisions.remove_turns);
    } else if is_responses_path(path) {
        apply_partial_turn_stubs_openai_responses(json, &decisions.partial_turn_stubs);
        for (turn, content) in &decisions.content_replacements {
            replace_openai_responses_content(json, *turn, content);
        }
        remove_openai_responses_items(json, &decisions.remove_turns);
    } else {
        warn!("Unknown API path for rewriting: {path}");
    }
}

/// Remove messages from an Anthropic request by turn index.
///
/// Turn index 0 = system (at `json["system"]`), 1+ = `messages[i-1]`.
pub(super) fn remove_anthropic_messages(json: &mut Value, turns_to_remove: &HashSet<u32>) {
    if turns_to_remove.is_empty() {
        return;
    }

    if turns_to_remove.contains(&0) {
        json.as_object_mut().map(|obj| obj.remove("system"));
    }

    if let Some(messages) = json.get_mut("messages").and_then(|v| v.as_array_mut()) {
        let indices_to_remove: HashSet<usize> = turns_to_remove
            .iter()
            .filter(|&&t| t > 0)
            .map(|&t| (t - 1) as usize)
            .collect();

        let mut indices: Vec<usize> = indices_to_remove.into_iter().collect();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in indices {
            if idx < messages.len() {
                messages.remove(idx);
            }
        }
    }
}

/// Replace content in an Anthropic message at a given turn index.
pub(super) fn replace_anthropic_content(json: &mut Value, turn: u32, new_content: &str) {
    if turn == 0 {
        if let Some(obj) = json.as_object_mut() {
            obj.insert("system".to_string(), Value::String(new_content.to_string()));
        }
        return;
    }

    let msg_idx = (turn - 1) as usize;
    if let Some(msg) = json
        .get_mut("messages")
        .and_then(|v| v.as_array_mut())
        .and_then(|arr| arr.get_mut(msg_idx))
    {
        replace_message_content(msg, new_content);
    }
}

/// Remove messages from an OpenAI Chat request by turn index.
pub(super) fn remove_openai_chat_messages(json: &mut Value, turns_to_remove: &HashSet<u32>) {
    if turns_to_remove.is_empty() {
        return;
    }

    if let Some(messages) = json.get_mut("messages").and_then(|v| v.as_array_mut()) {
        let mut indices: Vec<usize> = turns_to_remove
            .iter()
            .map(|&t| t as usize)
            .filter(|&i| i < messages.len())
            .collect();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in indices {
            messages.remove(idx);
        }
    }
}

/// Replace content in an OpenAI Chat message at a given turn index.
pub(super) fn replace_openai_chat_content(json: &mut Value, turn: u32, new_content: &str) {
    let idx = turn as usize;
    if let Some(msg) = json
        .get_mut("messages")
        .and_then(|v| v.as_array_mut())
        .and_then(|arr| arr.get_mut(idx))
    {
        replace_message_content(msg, new_content);
    }
}

/// Remove items from an OpenAI Responses request by turn index.
///
/// Turn index 0 = instructions, 1+ = `input[i-1]`.
pub(super) fn remove_openai_responses_items(json: &mut Value, turns_to_remove: &HashSet<u32>) {
    if turns_to_remove.is_empty() {
        return;
    }

    if turns_to_remove.contains(&0) {
        json.as_object_mut().map(|obj| obj.remove("instructions"));
    }

    if let Some(input) = json.get_mut("input").and_then(|v| v.as_array_mut()) {
        let mut indices: Vec<usize> = turns_to_remove
            .iter()
            .filter(|&&t| t > 0)
            .map(|&t| (t - 1) as usize)
            .filter(|&i| i < input.len())
            .collect();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in indices {
            input.remove(idx);
        }
    }
}

/// Replace content in an OpenAI Responses item at a given turn index.
pub(super) fn replace_openai_responses_content(json: &mut Value, turn: u32, new_content: &str) {
    if turn == 0 {
        if let Some(obj) = json.as_object_mut() {
            obj.insert(
                "instructions".to_string(),
                Value::String(new_content.to_string()),
            );
        }
        return;
    }

    let idx = (turn - 1) as usize;
    if let Some(item) = json
        .get_mut("input")
        .and_then(|v| v.as_array_mut())
        .and_then(|arr| arr.get_mut(idx))
    {
        replace_message_content(item, new_content);
    }
}

/// Replace the text content of a message/item JSON value.
fn replace_message_content(msg: &mut Value, new_content: &str) {
    if let Some(obj) = msg.as_object_mut() {
        match obj.get("content") {
            Some(Value::String(_)) => {
                obj.insert(
                    "content".to_string(),
                    Value::String(new_content.to_string()),
                );
            }
            Some(Value::Array(_)) => {
                if let Some(arr) = obj.get_mut("content").and_then(|v| v.as_array_mut()) {
                    for item in arr.iter_mut() {
                        if let Some(item_obj) = item.as_object_mut() {
                            let is_text = item_obj
                                .get("type")
                                .and_then(|v| v.as_str())
                                .map(|s| s == "text" || s == "input_text")
                                .unwrap_or(false);
                            if is_text {
                                item_obj.insert(
                                    "text".to_string(),
                                    Value::String(new_content.to_string()),
                                );
                                return;
                            }
                        }
                    }
                }
            }
            _ => {
                obj.insert(
                    "content".to_string(),
                    Value::String(new_content.to_string()),
                );
            }
        }
    }
}

// ── Partial-Turn Stub Application ───────────────────────────────

/// Replace individual content blocks within Anthropic messages with archival stubs.
///
/// For partial-turn archival: when not all blocks at a turn are archived,
/// we replace archived blocks' content with short stubs instead of removing
/// the entire message.
fn apply_partial_turn_stubs_anthropic(json: &mut Value, stubs: &[PartialTurnStub]) {
    if stubs.is_empty() {
        return;
    }

    for stub in stubs {
        if stub.turn_index == 0 {
            // System message — unlikely archival target, skip
            continue;
        }

        let msg_idx = (stub.turn_index - 1) as usize;
        let msg = match json
            .get_mut("messages")
            .and_then(|v| v.as_array_mut())
            .and_then(|arr| arr.get_mut(msg_idx))
        {
            Some(m) => m,
            None => continue,
        };

        // Handle content array (multi-block messages)
        if let Some(content_arr) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
            if let Some(block) = content_arr.get_mut(stub.content_index) {
                replace_content_block_with_stub(block, &stub.stub);
            }
        } else if stub.content_index == 0 {
            // String content (single block) — replace directly
            if let Some(obj) = msg.as_object_mut() {
                obj.insert("content".to_string(), Value::String(stub.stub.clone()));
            }
        }
    }
}

/// Replace individual content blocks within OpenAI Chat messages with archival stubs.
///
/// OpenAI Chat tool results are separate `role: "tool"` messages (their own turn),
/// so partial-turn stubs here mainly apply to assistant messages with both text and
/// tool_calls. For string-content messages, we replace the content directly.
fn apply_partial_turn_stubs_openai_chat(json: &mut Value, stubs: &[PartialTurnStub]) {
    if stubs.is_empty() {
        return;
    }

    for stub in stubs {
        let idx = stub.turn_index as usize;
        let msg = match json
            .get_mut("messages")
            .and_then(|v| v.as_array_mut())
            .and_then(|arr| arr.get_mut(idx))
        {
            Some(m) => m,
            None => continue,
        };

        // OpenAI Chat messages typically have string content
        if stub.content_index == 0 {
            if let Some(obj) = msg.as_object_mut() {
                if obj.get("content").is_some() {
                    obj.insert("content".to_string(), Value::String(stub.stub.clone()));
                }
            }
        }
    }
}

/// Replace individual items within OpenAI Responses `input[]` with archival stubs.
///
/// Responses API items are typically at their own turn_index, so partial-turn
/// stubs mainly apply when multiple items share a turn (rare).
fn apply_partial_turn_stubs_openai_responses(json: &mut Value, stubs: &[PartialTurnStub]) {
    if stubs.is_empty() {
        return;
    }

    for stub in stubs {
        if stub.turn_index == 0 {
            continue;
        }

        let idx = (stub.turn_index - 1) as usize;
        let item = match json
            .get_mut("input")
            .and_then(|v| v.as_array_mut())
            .and_then(|arr| arr.get_mut(idx))
        {
            Some(i) => i,
            None => continue,
        };

        // Replace output/content field
        if let Some(obj) = item.as_object_mut() {
            if obj.contains_key("output") {
                obj.insert("output".to_string(), Value::String(stub.stub.clone()));
            } else if obj.contains_key("content") {
                obj.insert("content".to_string(), Value::String(stub.stub.clone()));
            }
        }
    }
}

/// Replace the content of a single content block with a stub.
///
/// Handles Anthropic content block types:
/// - `tool_result`: replaces `content` field
/// - `text`: replaces `text` field
/// - `tool_use`: replaces `input` with empty object (tool call archived)
fn replace_content_block_with_stub(block: &mut Value, stub: &str) {
    let Some(obj) = block.as_object_mut() else {
        return;
    };

    let block_type = obj
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    match block_type.as_str() {
        "thinking" | "redacted_thinking" => {
            // Never modify thinking blocks — Anthropic requires byte-identical preservation
        }
        "tool_result" => {
            obj.insert("content".to_string(), Value::String(stub.to_string()));
        }
        "text" => {
            obj.insert("text".to_string(), Value::String(stub.to_string()));
        }
        "tool_use" => {
            obj.insert(
                "input".to_string(),
                serde_json::json!({"_archived": stub}),
            );
        }
        _ => {
            // Unknown type — try to replace content or text field
            if obj.contains_key("text") {
                obj.insert("text".to_string(), Value::String(stub.to_string()));
            } else if obj.contains_key("content") {
                obj.insert("content".to_string(), Value::String(stub.to_string()));
            }
        }
    }
}
