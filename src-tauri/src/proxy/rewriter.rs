//! JSON-level payload rewriting for context mutations.
//!
//! Applies `RewriteDecisions` to the raw JSON request body, working at the
//! `serde_json::Value` level to avoid lossy Block→JSON round-trips. Also handles
//! runtime cleanup (stripping context tools) and manifest injection.

use bytes::Bytes;
use serde_json::Value;
use tracing::{debug, warn};

use crate::engine::planner::applicator::{apply_mutations, EngineUpdateKind, RewriteDecisions};
use crate::engine::planner::file_tracker::{detect_file_mutations, FileMutation, ToolCallInfo};
use crate::engine::planner::types::PlannerInput;
use crate::engine::types::Role;
use crate::engine::ContextEngine;
use crate::metacog::{self, detect_runtime, RuntimeKind};
use crate::proxy::parser::{
    is_chat_completions_path, is_messages_path, is_responses_path, ParsedRequest,
};

/// Errors that can occur during payload rewriting.
#[derive(Debug, thiserror::Error)]
pub enum RewriteError {
    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("Rewrite failed: {0}")]
    Internal(String),
}

/// Rewrite a request payload with context mutations, cleanup, and manifest injection.
///
/// Returns `Ok(Some(rewritten_bytes))` if changes were made, `Ok(None)` if no
/// rewriting was needed (no engine, no mutations, no manifest).
pub fn rewrite_request(
    body: &[u8],
    path: &str,
    parsed: &ParsedRequest,
    engine: &ContextEngine,
) -> Result<Option<Bytes>, RewriteError> {
    // Get enriched blocks from engine
    let blocks = engine.active_session_blocks();
    if blocks.is_empty() {
        return Ok(None);
    }

    // Take pending plan and build planner input
    let pending_plan = engine.planner.take_pending_plan();
    let budget = engine.budget_status();
    let traffic_signals = collect_traffic_signals(path, body);
    let planner_signals = engine.planner.build_heuristic_signals(
        &blocks,
        &budget,
        traffic_signals.current_turn_files,
    );

    let input = PlannerInput {
        blocks: blocks.clone(),
        pending_plan,
        signals: planner_signals,
        budget: budget.clone(),
        file_mutations: if traffic_signals.file_mutations.is_empty() {
            None
        } else {
            Some(traffic_signals.file_mutations)
        },
    };

    // Run planner
    let plan_output = engine.planner.plan(&input);

    // Apply mutations to get rewrite decisions
    let decisions = apply_mutations(&blocks, &plan_output.mutations);

    // Detect runtime early (needed for gating decisions)
    let provider_str = parsed.provider.to_string();
    let runtime_kind = detect_runtime(path, &provider_str);

    // Determine if we need manifest injection (gated on context maturity)
    let manifest_text = plan_output.manifest.to_injection_text();
    let manifest_eligible = !manifest_text.is_empty() && should_inject_manifest(&blocks);
    let has_breadcrumb = plan_output.cleanup.breadcrumb.is_some();

    // Check if tools should be injected (non-passive, non-streaming, mature context)
    let should_inject =
        runtime_kind != RuntimeKind::Passive && !parsed.stream && should_inject_tools(&blocks);

    // If nothing to do, return None
    if !decisions.has_payload_changes() && !manifest_eligible && !has_breadcrumb && !should_inject {
        // Still apply engine-side updates even when payload is unchanged
        apply_engine_updates(engine, &decisions);
        debug!("No payload rewriting needed");
        return Ok(None);
    }

    // Parse body as JSON
    let mut json: Value = serde_json::from_slice(body)?;

    // Apply turn removals and content replacements
    if decisions.has_payload_changes() {
        apply_decisions_to_json(&mut json, path, &decisions);
        debug!(
            "Applied rewrite decisions: {} turns removed, {} content replacements",
            decisions.remove_turns.len(),
            decisions.content_replacements.len()
        );
    }

    // Select runtime for cleanup, manifest, and tool injection
    let runtime = metacog::select_runtime(runtime_kind, path);

    // Clean up context tool calls from history
    let cleanup_result = runtime.cleanup_history(&mut json);
    if cleanup_result.tool_uses_stripped > 0 {
        debug!(
            "Cleaned up {} context tool uses, {} results",
            cleanup_result.tool_uses_stripped, cleanup_result.tool_results_stripped
        );
    }

    // Inject manifest into system message
    if manifest_eligible {
        runtime.inject_manifest(&mut json, &manifest_text);
        debug!("Injected manifest into system message");
    }

    // Inject breadcrumb if mutations were applied
    if let Some(ref breadcrumb) = plan_output.cleanup.breadcrumb {
        runtime.inject_manifest(&mut json, breadcrumb);
        debug!("Injected breadcrumb into system message");
    }

    // Tool injection — gated on non-passive, non-streaming, and mature context
    if runtime_kind != RuntimeKind::Passive && !parsed.stream && should_inject_tools(&blocks) {
        runtime.inject_tools(&mut json);
        debug!("Injected context tools into request");
    }

    // Apply engine-side block updates (zone shifts, pins) from planner mutations
    apply_engine_updates(engine, &decisions);

    // Re-serialize
    let rewritten = serde_json::to_vec(&json)?;
    Ok(Some(Bytes::from(rewritten)))
}

/// Determine whether tools should be injected based on context maturity.
///
/// Tools are only injected when context is large enough that shifting becomes
/// relevant (more than 3 non-system blocks, i.e., at least 2 conversation turns).
pub(crate) fn should_inject_tools(blocks: &[crate::engine::block::Block]) -> bool {
    let non_system = blocks.iter().filter(|b| b.role != Role::System).count();
    non_system > 3
}

/// Determine whether manifest injection should happen.
///
/// Manifest is cheap (~30 tokens) but we skip it on truly empty contexts
/// (no non-system blocks at all).
pub(crate) fn should_inject_manifest(blocks: &[crate::engine::block::Block]) -> bool {
    blocks.iter().any(|b| b.role != Role::System)
}

/// Apply engine-side block updates from rewrite decisions.
fn apply_engine_updates(engine: &ContextEngine, decisions: &RewriteDecisions) {
    for update in &decisions.engine_updates {
        match &update.kind {
            EngineUpdateKind::SetZone(zone) => {
                engine.move_block_internal(&update.block_id, zone.clone());
            }
            EngineUpdateKind::SetPinned(pin) => {
                engine.set_pin_internal(&update.block_id, *pin);
            }
            EngineUpdateKind::Archive => {
                engine.archive_block_internal(&update.block_id);
            }
            EngineUpdateKind::ApplyCompression { summary } => {
                engine.apply_compression_summary_internal(&update.block_id, summary);
            }
            EngineUpdateKind::RestoreOriginal => {
                engine.restore_original_internal(&update.block_id);
            }
            EngineUpdateKind::UpdateContent { new_content } => {
                engine.update_content_internal(&update.block_id, new_content);
            }
        }
    }
    if !decisions.engine_updates.is_empty() {
        debug!(
            "Applied {} engine block updates",
            decisions.engine_updates.len()
        );
    }
}

#[derive(Debug, Default)]
struct TrafficSignals {
    current_turn_files: Vec<String>,
    file_mutations: Vec<FileMutation>,
}

#[derive(Debug, Clone)]
struct ToolCallEntry {
    turn_index: u32,
    name: String,
    arguments: Value,
    result: Option<String>,
}

fn collect_traffic_signals(path: &str, body: &[u8]) -> TrafficSignals {
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

// ── Format-specific JSON manipulation ────────────────────────────

/// Apply rewrite decisions to a JSON value, dispatching to the correct format.
fn apply_decisions_to_json(json: &mut Value, path: &str, decisions: &RewriteDecisions) {
    if is_messages_path(path) {
        remove_anthropic_messages(json, &decisions.remove_turns);
        for (turn, content) in &decisions.content_replacements {
            replace_anthropic_content(json, *turn, content);
        }
    } else if is_chat_completions_path(path) {
        remove_openai_chat_messages(json, &decisions.remove_turns);
        for (turn, content) in &decisions.content_replacements {
            replace_openai_chat_content(json, *turn, content);
        }
    } else if is_responses_path(path) {
        remove_openai_responses_items(json, &decisions.remove_turns);
        for (turn, content) in &decisions.content_replacements {
            replace_openai_responses_content(json, *turn, content);
        }
    } else {
        warn!("Unknown API path for rewriting: {path}");
    }
}

// ── Anthropic format ─────────────────────────────────────────────

/// Remove messages from an Anthropic request by turn index.
///
/// Turn index 0 = system (at `json["system"]`), 1+ = `messages[i-1]`.
fn remove_anthropic_messages(json: &mut Value, turns_to_remove: &HashSet<u32>) {
    if turns_to_remove.is_empty() {
        return;
    }

    // Handle system removal (turn 0)
    if turns_to_remove.contains(&0) {
        json.as_object_mut().map(|obj| obj.remove("system"));
    }

    // Remove messages (turn 1+ maps to messages[turn-1])
    if let Some(messages) = json.get_mut("messages").and_then(|v| v.as_array_mut()) {
        // Build set of message indices to remove (0-indexed)
        let indices_to_remove: HashSet<usize> = turns_to_remove
            .iter()
            .filter(|&&t| t > 0)
            .map(|&t| (t - 1) as usize)
            .collect();

        // Remove in reverse order to preserve indices
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
///
/// For simple string content, replaces directly. For array content blocks,
/// replaces the first text block's content.
fn replace_anthropic_content(json: &mut Value, turn: u32, new_content: &str) {
    if turn == 0 {
        // System message
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

// ── OpenAI Chat Completions format ───────────────────────────────

/// Remove messages from an OpenAI Chat request by turn index.
///
/// Turn index maps directly to `messages[turn]`.
fn remove_openai_chat_messages(json: &mut Value, turns_to_remove: &HashSet<u32>) {
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
fn replace_openai_chat_content(json: &mut Value, turn: u32, new_content: &str) {
    let idx = turn as usize;
    if let Some(msg) = json
        .get_mut("messages")
        .and_then(|v| v.as_array_mut())
        .and_then(|arr| arr.get_mut(idx))
    {
        replace_message_content(msg, new_content);
    }
}

// ── OpenAI Responses format ──────────────────────────────────────

/// Remove items from an OpenAI Responses request by turn index.
///
/// Turn index 0 = instructions, 1+ = `input[i-1]`.
fn remove_openai_responses_items(json: &mut Value, turns_to_remove: &HashSet<u32>) {
    if turns_to_remove.is_empty() {
        return;
    }

    // Handle instructions removal (turn 0)
    if turns_to_remove.contains(&0) {
        json.as_object_mut().map(|obj| obj.remove("instructions"));
    }

    // Remove input items (turn 1+ maps to input[turn-1])
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
fn replace_openai_responses_content(json: &mut Value, turn: u32, new_content: &str) {
    if turn == 0 {
        // Instructions
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

// ── Shared helper ────────────────────────────────────────────────

/// Replace the text content of a message/item JSON value.
///
/// Handles both string content (`"content": "text"`) and array content
/// (`"content": [{"type": "text", "text": "..."}]`) by replacing the first
/// text block found.
fn replace_message_content(msg: &mut Value, new_content: &str) {
    if let Some(obj) = msg.as_object_mut() {
        match obj.get("content") {
            // String content — replace directly
            Some(Value::String(_)) => {
                obj.insert(
                    "content".to_string(),
                    Value::String(new_content.to_string()),
                );
            }
            // Array content — replace first text block
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
                // No existing content — insert as string
                obj.insert(
                    "content".to_string(),
                    Value::String(new_content.to_string()),
                );
            }
        }
    }
}

use std::collections::HashSet;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::planner::file_tracker::FileMutationKind;
    use serde_json::json;

    // ── Planner signal extraction tests ──────────────────────

    #[test]
    fn test_collect_traffic_signals_responses_tracks_current_turn_files_and_mutations() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-4.1",
            "input": [
                {
                    "type": "function_call",
                    "name": "edit_file",
                    "call_id": "old_call",
                    "arguments": "{\"path\":\"src/old.rs\"}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "old_call",
                    "output": "old result"
                },
                {
                    "type": "function_call",
                    "name": "edit_file",
                    "call_id": "latest_call",
                    "arguments": "{\"path\":\"src/auth.rs\"}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "latest_call",
                    "output": "fn login() { updated }"
                }
            ]
        }))
        .expect("serialize request");

        let signals = collect_traffic_signals("/v1/responses", &body);
        assert_eq!(signals.current_turn_files, vec!["src/auth.rs".to_string()]);
        assert_eq!(signals.file_mutations.len(), 1);
        assert_eq!(signals.file_mutations[0].file_path, "src/auth.rs");
        assert_eq!(signals.file_mutations[0].kind, FileMutationKind::Edit);
    }

    #[test]
    fn test_collect_traffic_signals_chat_tracks_latest_tool_turn_only() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-4.1",
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "write_file",
                            "arguments": "{\"path\":\"src/old.rs\",\"content\":\"old\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "content": "old"
                },
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_2",
                        "type": "function",
                        "function": {
                            "name": "write_file",
                            "arguments": "{\"path\":\"src/new.rs\",\"content\":\"new\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_2",
                    "content": "new"
                }
            ]
        }))
        .expect("serialize request");

        let signals = collect_traffic_signals("/v1/chat/completions", &body);
        assert_eq!(signals.current_turn_files, vec!["src/new.rs".to_string()]);
        assert_eq!(signals.file_mutations.len(), 1);
        assert_eq!(signals.file_mutations[0].kind, FileMutationKind::Write);
    }

    // ── Anthropic format tests ────────────────────────────────────

    #[test]
    fn test_anthropic_remove_middle_message() {
        let mut json = json!({
            "model": "claude-sonnet-4-5-20250929",
            "system": "You are helpful.",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there"},
                {"role": "user", "content": "How are you?"}
            ]
        });

        // Turn 2 = messages[1] (assistant message)
        let turns = HashSet::from([2u32]);
        remove_anthropic_messages(&mut json, &turns);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "Hello");
        assert_eq!(messages[1]["content"], "How are you?");
    }

    #[test]
    fn test_anthropic_replace_content_string() {
        let mut json = json!({
            "model": "claude-sonnet-4-5-20250929",
            "messages": [
                {"role": "user", "content": "Old content"},
                {"role": "assistant", "content": "Response"}
            ]
        });

        replace_anthropic_content(&mut json, 1, "New content");

        assert_eq!(json["messages"][0]["content"], "New content");
    }

    #[test]
    fn test_anthropic_replace_content_array() {
        let mut json = json!({
            "model": "claude-sonnet-4-5-20250929",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "Old content"}
                ]}
            ]
        });

        replace_anthropic_content(&mut json, 1, "New content");

        assert_eq!(json["messages"][0]["content"][0]["text"], "New content");
    }

    #[test]
    fn test_anthropic_replace_system() {
        let mut json = json!({
            "model": "claude-sonnet-4-5-20250929",
            "system": "Old system prompt",
            "messages": []
        });

        replace_anthropic_content(&mut json, 0, "New system prompt");
        assert_eq!(json["system"], "New system prompt");
    }

    #[test]
    fn test_anthropic_remove_system() {
        let mut json = json!({
            "model": "claude-sonnet-4-5-20250929",
            "system": "System prompt",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let turns = HashSet::from([0u32]);
        remove_anthropic_messages(&mut json, &turns);

        assert!(json.get("system").is_none());
        assert_eq!(json["messages"].as_array().unwrap().len(), 1);
    }

    // ── OpenAI Chat format tests ──────────────────────────────────

    #[test]
    fn test_openai_chat_remove_message() {
        let mut json = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"}
            ]
        });

        // Remove turn 1 (user message)
        let turns = HashSet::from([1u32]);
        remove_openai_chat_messages(&mut json, &turns);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "assistant");
    }

    #[test]
    fn test_openai_chat_replace_content() {
        let mut json = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "System"},
                {"role": "user", "content": "Old"}
            ]
        });

        replace_openai_chat_content(&mut json, 1, "New user message");

        assert_eq!(json["messages"][1]["content"], "New user message");
    }

    // ── OpenAI Responses format tests ─────────────────────────────

    #[test]
    fn test_responses_remove_item() {
        let mut json = json!({
            "model": "gpt-4",
            "instructions": "Be helpful",
            "input": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"},
                {"role": "user", "content": "Bye"}
            ]
        });

        // Turn 2 = input[1] (assistant)
        let turns = HashSet::from([2u32]);
        remove_openai_responses_items(&mut json, &turns);

        let input = json["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["content"], "Hello");
        assert_eq!(input[1]["content"], "Bye");
    }

    #[test]
    fn test_responses_remove_instructions() {
        let mut json = json!({
            "model": "gpt-4",
            "instructions": "Be helpful",
            "input": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let turns = HashSet::from([0u32]);
        remove_openai_responses_items(&mut json, &turns);

        assert!(json.get("instructions").is_none());
        assert_eq!(json["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_responses_replace_content() {
        let mut json = json!({
            "model": "gpt-4",
            "input": [
                {"role": "user", "content": "Old message"}
            ]
        });

        replace_openai_responses_content(&mut json, 1, "New message");
        assert_eq!(json["input"][0]["content"], "New message");
    }

    #[test]
    fn test_responses_replace_instructions() {
        let mut json = json!({
            "model": "gpt-4",
            "instructions": "Old instructions",
            "input": []
        });

        replace_openai_responses_content(&mut json, 0, "New instructions");
        assert_eq!(json["instructions"], "New instructions");
    }

    // ── apply_decisions_to_json dispatch tests ────────────────────

    #[test]
    fn test_apply_decisions_anthropic_path() {
        let mut json = json!({
            "model": "claude-sonnet-4-5-20250929",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"},
                {"role": "user", "content": "Bye"}
            ]
        });

        let mut decisions = RewriteDecisions::default();
        decisions.remove_turns.insert(2); // Remove assistant
        decisions
            .content_replacements
            .insert(3, "Goodbye".to_string()); // Replace last user msg

        apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "Hello");
        // After removal of index 1, the "Bye" message shifted — but replacement uses original turn index
        // Turn 3 = messages[2] originally, but after removal it's messages[1]
        // The replacement happens BEFORE removal in the current implementation order,
        // so turn 3 = messages[2] is replaced, then turn 2 = messages[1] is removed
    }

    #[test]
    fn test_apply_decisions_openai_chat_path() {
        let mut json = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "System"},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"}
            ]
        });

        let mut decisions = RewriteDecisions::default();
        decisions.remove_turns.insert(1); // Remove user message

        apply_decisions_to_json(&mut json, "/v1/chat/completions", &decisions);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_apply_decisions_responses_path() {
        let mut json = json!({
            "model": "gpt-4",
            "instructions": "Be helpful",
            "input": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let mut decisions = RewriteDecisions::default();
        decisions
            .content_replacements
            .insert(0, "Updated instructions".to_string());

        apply_decisions_to_json(&mut json, "/v1/responses", &decisions);
        assert_eq!(json["instructions"], "Updated instructions");
    }

    // ── Edge cases ────────────────────────────────────────────────

    #[test]
    fn test_remove_multiple_turns_preserves_order() {
        let mut json = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "S"},
                {"role": "user", "content": "U1"},
                {"role": "assistant", "content": "A1"},
                {"role": "user", "content": "U2"},
                {"role": "assistant", "content": "A2"}
            ]
        });

        // Remove turns 1 and 3 (user messages)
        let turns = HashSet::from([1u32, 3u32]);
        remove_openai_chat_messages(&mut json, &turns);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["content"], "S");
        assert_eq!(messages[1]["content"], "A1");
        assert_eq!(messages[2]["content"], "A2");
    }

    #[test]
    fn test_empty_turns_no_changes() {
        let mut json = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let original = json.clone();
        remove_openai_chat_messages(&mut json, &HashSet::new());
        assert_eq!(json, original);
    }

    #[test]
    fn test_replace_content_with_array_input_text() {
        // OpenAI Responses uses "input_text" type
        let mut json = json!({
            "model": "gpt-4",
            "input": [
                {"role": "user", "content": [
                    {"type": "input_text", "text": "Old"}
                ]}
            ]
        });

        replace_openai_responses_content(&mut json, 1, "New");
        assert_eq!(json["input"][0]["content"][0]["text"], "New");
    }

    #[test]
    fn test_out_of_bounds_turn_index_ignored() {
        let mut json = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let turns = HashSet::from([99u32]);
        remove_openai_chat_messages(&mut json, &turns);
        // Should not crash
        assert_eq!(json["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_replace_out_of_bounds_turn_index_ignored() {
        let mut json = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        replace_openai_chat_content(&mut json, 99, "New");
        // Should not crash, original unchanged
        assert_eq!(json["messages"][0]["content"], "Hello");
    }

    // ── Tool injection gating tests ──────────────────────────

    #[test]
    fn test_should_inject_tools_empty_context() {
        let blocks: Vec<crate::engine::block::Block> = vec![];
        assert!(!should_inject_tools(&blocks));
    }

    #[test]
    fn test_should_inject_tools_small_context() {
        use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
        use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

        let make = |role: Role| Block {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            block_type: None,
            content: "test".to_string(),
            tokens: 10,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Recency),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "test".to_string(),
                    tokens: 10,
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
                provider: "test".to_string(),
                turn_index: 0,
                tool_name: None,
                file_paths: vec![],
            },
        };

        // 1 system + 2 non-system = 2 non-system, not enough
        let blocks = vec![make(Role::System), make(Role::User), make(Role::Assistant)];
        assert!(!should_inject_tools(&blocks));

        // 1 system + 4 non-system = 4 non-system, enough (> 3)
        let blocks = vec![
            make(Role::System),
            make(Role::User),
            make(Role::Assistant),
            make(Role::User),
            make(Role::Assistant),
        ];
        assert!(should_inject_tools(&blocks));
    }

    #[test]
    fn test_should_inject_manifest_empty_context() {
        let blocks: Vec<crate::engine::block::Block> = vec![];
        assert!(!should_inject_manifest(&blocks));
    }

    #[test]
    fn test_should_inject_manifest_system_only() {
        use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
        use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

        let system_block = Block {
            id: "sys".to_string(),
            role: Role::System,
            block_type: None,
            content: "system prompt".to_string(),
            tokens: 10,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Primacy),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "system prompt".to_string(),
                    tokens: 10,
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
                provider: "test".to_string(),
                turn_index: 0,
                tool_name: None,
                file_paths: vec![],
            },
        };

        // System-only context: no manifest
        assert!(!should_inject_manifest(&[system_block.clone()]));

        // System + user: manifest eligible
        let mut user_block = system_block.clone();
        user_block.role = Role::User;
        user_block.id = "user".to_string();
        assert!(should_inject_manifest(&[system_block, user_block]));
    }
}
