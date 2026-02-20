use serde_json::Value;

use crate::engine::planner::file_tracker::{detect_file_mutations, FileMutation, ToolCallInfo};
use crate::proxy::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

#[derive(Debug, Default)]
pub(super) struct TrafficSignals {
    pub(super) current_turn_files: Vec<String>,
    pub(super) file_mutations: Vec<FileMutation>,
}

#[derive(Debug, Clone)]
struct ToolCallEntry {
    turn_index: u32,
    name: String,
    arguments: Value,
    result: Option<String>,
}

pub(super) fn collect_traffic_signals(path: &str, body: &[u8]) -> TrafficSignals {
    let json: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return TrafficSignals::default(),
    };

    let entries = extract_tool_call_entries(path, &json);
    if entries.is_empty() {
        return TrafficSignals::default();
    }

    let max_turn = entries
        .iter()
        .map(|entry| entry.turn_index)
        .max()
        .unwrap_or(0);
    let current_turn_calls: Vec<ToolCallInfo> = entries
        .iter()
        .filter(|entry| entry.turn_index == max_turn)
        .map(|entry| ToolCallInfo {
            name: entry.name.clone(),
            arguments: entry.arguments.clone(),
            result: entry.result.clone(),
        })
        .collect();
    let file_mutations = detect_file_mutations(&current_turn_calls);

    let mut current_turn_files: Vec<String> = current_turn_calls
        .iter()
        .filter_map(|call| extract_file_path_from_args(&call.arguments))
        .collect();
    current_turn_files.extend(file_mutations.iter().map(|m| m.file_path.clone()));
    current_turn_files.sort_unstable();
    current_turn_files.dedup();

    TrafficSignals {
        current_turn_files,
        file_mutations,
    }
}

fn extract_tool_call_entries(path: &str, json: &Value) -> Vec<ToolCallEntry> {
    if is_responses_path(path) {
        extract_responses_tool_call_entries(json)
    } else if is_chat_completions_path(path) {
        extract_chat_tool_call_entries(json)
    } else if is_messages_path(path) {
        extract_anthropic_tool_call_entries(json)
    } else {
        Vec::new()
    }
}

fn extract_responses_tool_call_entries(json: &Value) -> Vec<ToolCallEntry> {
    use std::collections::HashMap;

    let mut pending_calls: HashMap<String, (u32, String, Value)> = HashMap::new();
    let mut call_results: HashMap<String, (u32, String)> = HashMap::new();

    if let Some(items) = json.get("input").and_then(|v| v.as_array()) {
        for (idx, item) in items.iter().enumerate() {
            let turn_index = idx as u32;
            let item_type = item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            match item_type {
                "function_call" => {
                    let Some(call_id) = item.get("call_id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .and_then(parse_arguments)
                        .unwrap_or(Value::Object(Default::default()));
                    pending_calls.insert(
                        call_id.to_string(),
                        (turn_index, name.to_string(), arguments),
                    );
                }
                "function_call_output" => {
                    let Some(call_id) = item.get("call_id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let result = item
                        .get("output")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    call_results.insert(call_id.to_string(), (turn_index, result));
                }
                _ => {}
            }
        }
    }

    build_entries_from_calls(pending_calls, call_results)
}

fn extract_chat_tool_call_entries(json: &Value) -> Vec<ToolCallEntry> {
    use std::collections::HashMap;

    let mut pending_calls: HashMap<String, (u32, String, Value)> = HashMap::new();
    let mut call_results: HashMap<String, (u32, String)> = HashMap::new();

    if let Some(messages) = json.get("messages").and_then(|v| v.as_array()) {
        for (idx, message) in messages.iter().enumerate() {
            let turn_index = idx as u32;
            if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                for call in tool_calls {
                    let Some(call_id) = call.get("id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let Some(function) = call.get("function").and_then(|v| v.as_object()) else {
                        continue;
                    };
                    let Some(name) = function.get("name").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let arguments = function
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .and_then(parse_arguments)
                        .unwrap_or(Value::Object(Default::default()));
                    pending_calls.insert(
                        call_id.to_string(),
                        (turn_index, name.to_string(), arguments),
                    );
                }
            }

            if message.get("role").and_then(|v| v.as_str()) == Some("tool") {
                let Some(call_id) = message.get("tool_call_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let result = extract_message_text(message.get("content")).unwrap_or_default();
                call_results.insert(call_id.to_string(), (turn_index, result));
            }
        }
    }

    build_entries_from_calls(pending_calls, call_results)
}

fn extract_anthropic_tool_call_entries(json: &Value) -> Vec<ToolCallEntry> {
    use std::collections::HashMap;

    let mut pending_calls: HashMap<String, (u32, String, Value)> = HashMap::new();
    let mut call_results: HashMap<String, (u32, String)> = HashMap::new();

    if let Some(messages) = json.get("messages").and_then(|v| v.as_array()) {
        for (idx, message) in messages.iter().enumerate() {
            let turn_index = (idx + 1) as u32;
            let Some(content_blocks) = message.get("content").and_then(|v| v.as_array()) else {
                continue;
            };
            for block in content_blocks {
                let block_type = block
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                match block_type {
                    "tool_use" => {
                        let Some(call_id) = block.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        let Some(name) = block.get("name").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        let arguments = block
                            .get("input")
                            .cloned()
                            .unwrap_or(Value::Object(Default::default()));
                        pending_calls.insert(
                            call_id.to_string(),
                            (turn_index, name.to_string(), arguments),
                        );
                    }
                    "tool_result" => {
                        let Some(call_id) = block.get("tool_use_id").and_then(|v| v.as_str())
                        else {
                            continue;
                        };
                        let result = extract_message_text(block.get("content")).unwrap_or_default();
                        call_results.insert(call_id.to_string(), (turn_index, result));
                    }
                    _ => {}
                }
            }
        }
    }

    build_entries_from_calls(pending_calls, call_results)
}

fn build_entries_from_calls(
    pending_calls: std::collections::HashMap<String, (u32, String, Value)>,
    call_results: std::collections::HashMap<String, (u32, String)>,
) -> Vec<ToolCallEntry> {
    let mut entries = Vec::new();
    for (call_id, (call_turn, name, arguments)) in pending_calls {
        let (result_turn, result) = call_results
            .get(&call_id)
            .map(|(turn, text)| (*turn, Some(text.clone())))
            .unwrap_or((call_turn, None));
        entries.push(ToolCallEntry {
            turn_index: call_turn.max(result_turn),
            name,
            arguments,
            result,
        });
    }
    entries
}

fn parse_arguments(raw: &str) -> Option<Value> {
    serde_json::from_str(raw).ok()
}

fn extract_message_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        other => Some(other.to_string()),
    }
}

fn extract_file_path_from_args(arguments: &Value) -> Option<String> {
    let object = arguments.as_object()?;
    [
        "file_path",
        "path",
        "filename",
        "file",
        "filePath",
        "file_name",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(|v| v.as_str()))
    .map(ToString::to_string)
}
