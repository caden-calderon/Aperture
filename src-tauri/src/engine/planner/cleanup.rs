//! Ephemeral cleanup system — strips context tool calls and generates breadcrumbs.
//!
//! Between turns, context tool entries are stripped from conversation history
//! and replaced with a breadcrumb summary. Two categories:
//!
//! 1. **Intercepted tools** (`aperture_context_*`): Proxy-injected, handled by
//!    the interceptor. Stripped from ALL turns unconditionally.
//!
//! 2. **MCP tools** (`mcp__aperture__aperture_context_*`): Called by the model
//!    via MCP server, appear as regular tool_use/tool_result in history.
//!    Stripped from OLDER turns only — the most recent tool cycle is preserved
//!    so the model can process its own tool results.
//!
//! The "recent" boundary is the LAST assistant message (Anthropic/OpenAI Chat)
//! or the position after the last assistant text message (Responses API).
//!
//! Edge cases handled by this design:
//! - If the model quotes an old tool result, the text lives in the model's own
//!   response (preserved), not in the stripped tool_result block.
//! - Multi-turn tool chaining: intermediate results are stripped once the model
//!   generates a response. The model's response carries the semantics forward.
//! - Structural artifacts from stripping (consecutive roles, orphan blocks) are
//!   handled by the rewriter's sanitization pipeline that runs after cleanup.
//!
//! Supports Anthropic, OpenAI Chat Completions, and OpenAI Responses API formats.

use std::collections::HashSet;

use serde_json::Value;

use crate::metacog::runtime::{
    is_context_tool_name, is_intercepted_context_tool_name, CleanupResult,
};

use super::types::ContextMutation;

// ── Breadcrumb Generation ──────────────────────────────────────

/// Generate a breadcrumb summary from applied mutations and budget state.
///
/// Format: `[Context update: expanded #8 → primacy, archived #12. Net: -1,960 tok. Budget: 52%]`
pub fn generate_breadcrumb(
    mutations: &[ContextMutation],
    net_token_delta: i64,
    budget_pct: f64,
) -> String {
    if mutations.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();

    for mutation in mutations {
        let desc = match mutation {
            ContextMutation::Expand { block_id } => format!("expanded #{block_id}"),
            ContextMutation::Archive { block_id } => format!("archived #{block_id}"),
            ContextMutation::Recall { block_id } => format!("recalled #{block_id}"),
            ContextMutation::Pin { block_id } => format!("pinned #{block_id}"),
            ContextMutation::Unpin { block_id } => format!("unpinned #{block_id}"),
            ContextMutation::Shift {
                block_id,
                target_zone,
            } => format!("shifted #{block_id} → {target_zone:?}"),
            ContextMutation::Compress {
                block_id,
                summary: _,
            } => format!("compressed #{block_id}"),
            ContextMutation::Split {
                thread_id, at_turn, ..
            } => format!("split {thread_id} at turn {at_turn}"),
            ContextMutation::UpdateContent { block_id, .. } => {
                format!("updated #{block_id}")
            }
        };
        parts.push(desc);
    }

    let actions = parts.join(", ");
    let delta_str = format_token_delta(net_token_delta);
    let pct = (budget_pct * 100.0).round() as i32;

    format!("[Context update: {actions}. Net: {delta_str}. Budget: {pct}%]")
}

/// Format a token delta for display (e.g. "+1.2k", "-960").
fn format_token_delta(delta: i64) -> String {
    let sign = if delta >= 0 { "+" } else { "-" };
    let abs = delta.unsigned_abs();
    if abs >= 1000 {
        let k = abs as f64 / 1000.0;
        if k >= 10.0 {
            format!("{sign}{}k", k.round() as u64)
        } else {
            format!("{sign}{k:.1}k")
        }
    } else {
        format!("{sign}{abs}")
    }
}

// ── Anthropic Format Cleanup ───────────────────────────────────

/// Strip `aperture_context_*` tool calls from Anthropic-format messages.
///
/// Anthropic messages use:
/// - `content[].type = "tool_use"` with `name` field in assistant messages
/// - `content[].type = "tool_result"` with `tool_use_id` field in user messages
///
/// Returns the set of stripped tool_use IDs for breadcrumb correlation.
pub fn strip_anthropic_context_tools(messages: &mut Value) -> CleanupResult {
    let msgs = match messages.as_array_mut() {
        Some(arr) => arr,
        None => return CleanupResult::default(),
    };

    // Find the last assistant message. MCP context tools in this message are
    // the model's most recent tool cycle — must be preserved so it can process
    // its own tool results. Tools in earlier messages are stale.
    let last_assistant_idx = msgs.iter().rposition(|msg| {
        msg.get("role").and_then(|r| r.as_str()) == Some("assistant")
    });

    // First pass: collect tool_use IDs to strip.
    // - Intercepted (proxy-injected) tools: strip from ALL assistant messages
    // - MCP context tools: strip only from NON-LAST assistant messages
    let mut ids_to_strip: HashSet<String> = HashSet::new();

    for (idx, msg) in msgs.iter().enumerate() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let content = match msg.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        let is_last_assistant = last_assistant_idx == Some(idx);

        for block in content {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            let name = match block.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };
            let id = match block.get("id").and_then(|i| i.as_str()) {
                Some(i) => i,
                None => continue,
            };

            // Strip intercepted tools (always) and stale MCP context tools
            // (from earlier assistant messages — model already consumed those).
            let should_strip = is_intercepted_context_tool_name(name)
                || (is_context_tool_name(name) && !is_last_assistant);
            if should_strip {
                ids_to_strip.insert(id.to_string());
            }
        }
    }

    if ids_to_strip.is_empty() {
        return CleanupResult::default();
    }

    let mut tool_uses_stripped = 0usize;
    let mut tool_results_stripped = 0usize;

    // Second pass: strip tool_use blocks by ID from assistant messages,
    // and matching tool_result blocks from user messages.
    for msg in msgs.iter_mut() {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();

        let content = match msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            Some(arr) => arr,
            None => continue,
        };

        match role.as_str() {
            "assistant" => {
                let before = content.len();
                content.retain(|block| {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                            if ids_to_strip.contains(id) {
                                return false; // Strip
                            }
                        }
                    }
                    true
                });
                tool_uses_stripped += before - content.len();
            }
            "user" => {
                let before = content.len();
                content.retain(|block| {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        if let Some(id) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                            if ids_to_strip.contains(id) {
                                return false; // Strip
                            }
                        }
                    }
                    true
                });
                tool_results_stripped += before - content.len();
            }
            _ => {}
        }
    }

    // Third pass: remove messages with empty content arrays.
    msgs.retain(|msg| {
        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
            !content.is_empty()
        } else {
            true // Keep non-array content messages (e.g. string content)
        }
    });

    CleanupResult {
        tool_uses_stripped,
        tool_results_stripped,
        breadcrumb: None, // Caller attaches breadcrumb after cleanup
    }
}

// ── OpenAI Format Cleanup ──────────────────────────────────────

/// Strip `aperture_context_*` tool calls from OpenAI-format messages.
///
/// OpenAI messages use:
/// - `tool_calls[]` array on assistant messages with `function.name`
/// - Separate `role: "tool"` messages with `tool_call_id`
///
/// Works for both Chat Completions (`messages[]`) and Responses API (`input[]`).
pub fn strip_openai_context_tools(messages: &mut Value) -> CleanupResult {
    let msgs = match messages.as_array_mut() {
        Some(arr) => arr,
        None => return CleanupResult::default(),
    };

    // Find the last assistant message — MCP context tools here are "recent".
    let last_assistant_idx = msgs.iter().rposition(|msg| {
        msg.get("role").and_then(|r| r.as_str()) == Some("assistant")
    });

    // First pass: collect tool_call IDs to strip.
    let mut ids_to_strip: HashSet<String> = HashSet::new();

    for (idx, msg) in msgs.iter().enumerate() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "assistant" {
            continue;
        }
        let tool_calls = match msg.get("tool_calls").and_then(|tc| tc.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        let is_last_assistant = last_assistant_idx == Some(idx);

        for tc in tool_calls {
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let id = match tc.get("id").and_then(|i| i.as_str()) {
                Some(i) => i,
                None => continue,
            };

            let should_strip = is_intercepted_context_tool_name(name)
                || (is_context_tool_name(name) && !is_last_assistant);
            if should_strip {
                ids_to_strip.insert(id.to_string());
            }
        }
    }

    if ids_to_strip.is_empty() {
        return CleanupResult::default();
    }

    let mut tool_uses_stripped = 0usize;
    let mut tool_results_stripped = 0usize;

    // Second pass: strip tool calls by ID from assistant messages.
    for msg in msgs.iter_mut() {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();

        if role == "assistant" {
            if let Some(tool_calls) = msg.get_mut("tool_calls").and_then(|tc| tc.as_array_mut()) {
                let before = tool_calls.len();
                tool_calls.retain(|tc| {
                    if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                        !ids_to_strip.contains(id)
                    } else {
                        true
                    }
                });
                tool_uses_stripped += before - tool_calls.len();

                // Remove the tool_calls key entirely if empty
                if tool_calls.is_empty() {
                    if let Some(obj) = msg.as_object_mut() {
                        obj.remove("tool_calls");
                    }
                }
            }
        }
    }

    // Third pass: remove tool result messages that match stripped IDs.
    msgs.retain(|msg| {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "tool" {
            if let Some(id) = msg.get("tool_call_id").and_then(|i| i.as_str()) {
                if ids_to_strip.contains(id) {
                    tool_results_stripped += 1;
                    return false; // Strip
                }
            }
        }
        true
    });

    // Fourth pass: remove assistant messages with no content and no tool_calls.
    msgs.retain(|msg| {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "assistant" {
            let has_content = msg
                .get("content")
                .map(|c| !c.is_null() && c.as_str().map(|s| !s.is_empty()).unwrap_or(true))
                .unwrap_or(false);
            let has_tool_calls = msg
                .get("tool_calls")
                .and_then(|tc| tc.as_array())
                .map(|arr| !arr.is_empty())
                .unwrap_or(false);
            has_content || has_tool_calls
        } else {
            true
        }
    });

    CleanupResult {
        tool_uses_stripped,
        tool_results_stripped,
        breadcrumb: None,
    }
}

// ── OpenAI Responses API Format Cleanup ────────────────────────

/// Strip `aperture_context_*` function calls from OpenAI Responses API `input[]`.
///
/// Responses API uses:
/// - `{"type": "function_call", "name": "...", "call_id": "..."}` for tool calls
/// - `{"type": "function_call_output", "call_id": "..."}` for tool results
pub fn strip_openai_responses_context_tools(input: &mut Value) -> CleanupResult {
    let items = match input.as_array_mut() {
        Some(arr) => arr,
        None => return CleanupResult::default(),
    };

    // Find the position of the last assistant text message. MCP context
    // function_calls AFTER this position are "recent" (model hasn't generated
    // a text response yet); those before are "stale" (model already consumed).
    // If no assistant message exists, all function_calls are considered recent
    // (conservative: preserve when uncertain).
    let last_assistant_msg_idx = items.iter().rposition(|item| {
        item.get("type").and_then(|t| t.as_str()) == Some("message")
            && item.get("role").and_then(|r| r.as_str()) == Some("assistant")
    });

    // Collect function call IDs to strip.
    let mut ids_to_strip: HashSet<String> = HashSet::new();

    for (idx, item) in items.iter().enumerate() {
        if item.get("type").and_then(|t| t.as_str()) != Some("function_call") {
            continue;
        }
        let name = match item.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };
        let id = match item.get("call_id").and_then(|i| i.as_str()) {
            Some(i) => i,
            None => continue,
        };

        // Strip intercepted tools (always) and stale MCP context tools
        // (before the last assistant message — model already processed those).
        let is_stale_mcp = is_context_tool_name(name)
            && last_assistant_msg_idx.is_some_and(|last_idx| idx < last_idx);
        if is_intercepted_context_tool_name(name) || is_stale_mcp {
            ids_to_strip.insert(id.to_string());
        }
    }

    if ids_to_strip.is_empty() {
        return CleanupResult::default();
    }

    let mut tool_uses_stripped = 0usize;
    let mut tool_results_stripped = 0usize;

    items.retain(|item| {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match item_type {
            "function_call" => {
                if let Some(id) = item.get("call_id").and_then(|i| i.as_str()) {
                    if ids_to_strip.contains(id) {
                        tool_uses_stripped += 1;
                        return false;
                    }
                }
                true
            }
            "function_call_output" => {
                if let Some(id) = item.get("call_id").and_then(|i| i.as_str()) {
                    if ids_to_strip.contains(id) {
                        tool_results_stripped += 1;
                        return false;
                    }
                }
                true
            }
            _ => true,
        }
    });

    CleanupResult {
        tool_uses_stripped,
        tool_results_stripped,
        breadcrumb: None,
    }
}

// ── Manifest Injection Helpers ─────────────────────────────────

/// Inject manifest text into an Anthropic-format request's system field.
///
/// Anthropic has a top-level `system` key (string or content array).
pub fn inject_manifest_anthropic(request_json: &mut Value, manifest: &str) {
    if manifest.is_empty() {
        return;
    }

    match request_json.get("system") {
        Some(Value::String(existing)) => {
            let new_system = format!("{manifest}\n\n{existing}");
            request_json["system"] = Value::String(new_system);
        }
        Some(Value::Array(_)) => {
            // Content block array — prepend a text block.
            if let Some(arr) = request_json
                .get_mut("system")
                .and_then(|s| s.as_array_mut())
            {
                arr.insert(
                    0,
                    serde_json::json!({
                        "type": "text",
                        "text": manifest
                    }),
                );
            }
        }
        _ => {
            // No system field — create one.
            request_json["system"] = Value::String(manifest.to_string());
        }
    }
}

/// Inject manifest text into an OpenAI-format request's system message.
///
/// OpenAI uses `role: "system"` as first message in `messages[]` or `input[]`.
pub fn inject_manifest_openai(messages: &mut Value, manifest: &str) {
    if manifest.is_empty() {
        return;
    }

    let msgs = match messages.as_array_mut() {
        Some(arr) => arr,
        None => return,
    };

    // Find existing system message and prepend.
    if let Some(first) = msgs.first_mut() {
        if first.get("role").and_then(|r| r.as_str()) == Some("system") {
            if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
                let new_content = format!("{manifest}\n\n{content}");
                first["content"] = Value::String(new_content);
                return;
            }
        }
    }

    // No system message — insert one at the front.
    msgs.insert(
        0,
        serde_json::json!({
            "role": "system",
            "content": manifest
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::BuiltInZone;

    // ── Breadcrumb Tests ─────────────────────────────────────

    #[test]
    fn test_generate_breadcrumb_empty_mutations() {
        let result = generate_breadcrumb(&[], 0, 0.5);
        assert!(result.is_empty());
    }

    #[test]
    fn test_generate_breadcrumb_single_archive() {
        let mutations = vec![ContextMutation::Archive {
            block_id: "12".into(),
        }];
        let result = generate_breadcrumb(&mutations, -1960, 0.52);
        assert_eq!(
            result,
            "[Context update: archived #12. Net: -2.0k. Budget: 52%]"
        );
    }

    #[test]
    fn test_generate_breadcrumb_multiple_mutations() {
        let mutations = vec![
            ContextMutation::Expand {
                block_id: "8".into(),
            },
            ContextMutation::Archive {
                block_id: "12".into(),
            },
            ContextMutation::Pin {
                block_id: "3".into(),
            },
        ];
        let result = generate_breadcrumb(&mutations, -500, 0.45);
        assert!(result.contains("expanded #8"));
        assert!(result.contains("archived #12"));
        assert!(result.contains("pinned #3"));
        assert!(result.contains("Net: -500"));
        assert!(result.contains("Budget: 45%"));
    }

    #[test]
    fn test_generate_breadcrumb_shift() {
        let mutations = vec![ContextMutation::Shift {
            block_id: "5".into(),
            target_zone: BuiltInZone::Primacy,
        }];
        let result = generate_breadcrumb(&mutations, 0, 0.60);
        assert!(result.contains("shifted #5"));
        assert!(result.contains("Primacy"));
    }

    #[test]
    fn test_format_token_delta_positive() {
        assert_eq!(format_token_delta(500), "+500");
        assert_eq!(format_token_delta(1500), "+1.5k");
        assert_eq!(format_token_delta(12000), "+12k");
    }

    #[test]
    fn test_format_token_delta_negative() {
        assert_eq!(format_token_delta(-960), "-960");
        assert_eq!(format_token_delta(-1960), "-2.0k");
        assert_eq!(format_token_delta(-15000), "-15k");
    }

    #[test]
    fn test_format_token_delta_zero() {
        assert_eq!(format_token_delta(0), "+0");
    }

    // ── Anthropic Cleanup Tests ──────────────────────────────

    #[test]
    fn test_strip_anthropic_no_context_tools() {
        let mut messages = serde_json::json!([
            {
                "role": "user",
                "content": [{"type": "text", "text": "Hello"}]
            },
            {
                "role": "assistant",
                "content": [{"type": "text", "text": "Hi there"}]
            }
        ]);

        let result = strip_anthropic_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 0);
        assert_eq!(result.tool_results_stripped, 0);
        assert_eq!(messages.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_strip_anthropic_context_tool_use_and_result() {
        let mut messages = serde_json::json!([
            {
                "role": "user",
                "content": [{"type": "text", "text": "Help me"}]
            },
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Let me check..."},
                    {"type": "tool_use", "id": "toolu_1", "name": "aperture_context_preview", "input": {}}
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "Preview data..."}
                ]
            },
            {
                "role": "assistant",
                "content": [{"type": "text", "text": "Based on the context..."}]
            }
        ]);

        let result = strip_anthropic_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = messages.as_array().unwrap();
        // tool_result-only user message should be removed (empty content)
        assert_eq!(msgs.len(), 3);
        // Assistant message should still have text but not tool_use
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(assistant_content.len(), 1);
        assert_eq!(assistant_content[0]["type"], "text");
    }

    #[test]
    fn test_strip_anthropic_mcp_namespaced_tools_are_preserved() {
        let mut messages = serde_json::json!([
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Checking context..."},
                    {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "mcp__aperture__aperture_context_preview",
                        "input": {}
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "Context preview..."}
                ]
            }
        ]);

        let result = strip_anthropic_context_tools(&mut messages);
        assert_eq!(
            result.tool_uses_stripped, 0,
            "MCP tools in the last assistant message are recent — must be preserved"
        );
        assert_eq!(result.tool_results_stripped, 0);
        // Both messages preserved — model's most recent tool cycle
        let msgs = messages.as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        let assistant_content = msgs[0]["content"].as_array().unwrap();
        assert_eq!(assistant_content.len(), 2);
        assert_eq!(assistant_content[1]["name"], "mcp__aperture__aperture_context_preview");
    }

    /// Fix 3: Stale MCP context tools (from earlier assistant messages) ARE stripped.
    /// Only the most recent tool cycle (last assistant message) is preserved.
    #[test]
    fn test_strip_anthropic_stale_mcp_tools_stripped() {
        let mut messages = serde_json::json!([
            // Turn 1: model calls MCP preview (stale — processed in next assistant msg)
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Let me check the context..."},
                    {
                        "type": "tool_use",
                        "id": "toolu_old",
                        "name": "mcp__aperture__aperture_context_preview",
                        "input": {}
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_old", "content": "Blocks: [1, 2, 3]..."}
                ]
            },
            // Turn 2: model processed the result and responds
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Based on the context, I see 3 blocks. Let me plan archival..."},
                    {
                        "type": "tool_use",
                        "id": "toolu_recent",
                        "name": "mcp__aperture__aperture_context_plan",
                        "input": {"archive": ["1"]}
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_recent", "content": "Plan staged."}
                ]
            }
        ]);

        let result = strip_anthropic_context_tools(&mut messages);

        // Stale preview (toolu_old) should be stripped
        assert_eq!(result.tool_uses_stripped, 1, "stale MCP tool_use should be stripped");
        assert_eq!(result.tool_results_stripped, 1, "stale MCP tool_result should be stripped");

        let msgs = messages.as_array().unwrap();
        // Turn 1 assistant keeps text, turn 1 user (tool_result only) removed,
        // turn 2 assistant + user preserved
        assert_eq!(msgs.len(), 3);

        // First assistant: only text remains (tool_use stripped)
        let first_assistant = msgs[0]["content"].as_array().unwrap();
        assert_eq!(first_assistant.len(), 1);
        assert_eq!(first_assistant[0]["type"], "text");

        // Last assistant: text + recent MCP tool_use preserved
        let last_assistant = msgs[1]["content"].as_array().unwrap();
        assert_eq!(last_assistant.len(), 2);
        assert_eq!(last_assistant[1]["name"], "mcp__aperture__aperture_context_plan");

        // Last user: recent tool_result preserved
        let last_user = msgs[2]["content"].as_array().unwrap();
        assert_eq!(last_user.len(), 1);
        assert_eq!(last_user[0]["tool_use_id"], "toolu_recent");
    }

    /// When all tool cycles are complete (last message is regular user text),
    /// ALL MCP context tools should be stripped.
    #[test]
    fn test_strip_anthropic_all_mcp_stripped_when_processed() {
        let mut messages = serde_json::json!([
            // Model called MCP preview
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "mcp__aperture__aperture_context_preview",
                        "input": {}
                    }
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "Preview..."}
                ]
            },
            // Model processed result and responded with text (no new tool calls)
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "I see the context. Everything looks good."}
                ]
            },
            // User sends regular follow-up
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "Great, now write some code."}
                ]
            }
        ]);

        let result = strip_anthropic_context_tools(&mut messages);

        // The preview is stale (model responded at turn 2 with text, no tool_use)
        assert_eq!(result.tool_uses_stripped, 1, "processed MCP tool_use should be stripped");
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = messages.as_array().unwrap();
        // First assistant becomes empty → removed. Tool_result user → removed.
        // Remaining: assistant (text) + user (text)
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["content"][0]["text"], "I see the context. Everything looks good.");
        assert_eq!(msgs[1]["content"][0]["text"], "Great, now write some code.");
    }

    #[test]
    fn test_strip_anthropic_mixed_real_and_context_tools() {
        let mut messages = serde_json::json!([
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "I'll search and check context..."},
                    {"type": "tool_use", "id": "toolu_1", "name": "aperture_context_preview", "input": {}},
                    {"type": "tool_use", "id": "toolu_2", "name": "read_file", "input": {"path": "foo.rs"}}
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "Context preview..."},
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": "File contents..."}
                ]
            }
        ]);

        let result = strip_anthropic_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = messages.as_array().unwrap();
        assert_eq!(msgs.len(), 2);

        // Assistant should keep text + real tool_use
        let assistant_content = msgs[0]["content"].as_array().unwrap();
        assert_eq!(assistant_content.len(), 2);
        assert_eq!(assistant_content[1]["name"], "read_file");

        // User should keep real tool_result
        let user_content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(user_content.len(), 1);
        assert_eq!(user_content[0]["tool_use_id"], "toolu_2");
    }

    #[test]
    fn test_strip_anthropic_multiple_context_tools() {
        let mut messages = serde_json::json!([
            {
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "aperture_context_preview", "input": {}},
                    {"type": "tool_use", "id": "toolu_2", "name": "aperture_context_search", "input": {"query": "auth"}}
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "Preview..."},
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": "Search results..."}
                ]
            },
            {
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "toolu_3", "name": "aperture_context_plan", "input": {"archive": ["5"]}}
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_3", "content": "Plan validated..."}
                ]
            }
        ]);

        let result = strip_anthropic_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 3);
        assert_eq!(result.tool_results_stripped, 3);
        // All messages should be removed (all content arrays become empty)
        assert_eq!(messages.as_array().unwrap().len(), 0);
    }

    // ── OpenAI Chat Cleanup Tests ────────────────────────────

    #[test]
    fn test_strip_openai_no_context_tools() {
        let mut messages = serde_json::json!([
            {"role": "system", "content": "You are helpful"},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"}
        ]);

        let result = strip_openai_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 0);
        assert_eq!(result.tool_results_stripped, 0);
        assert_eq!(messages.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_strip_openai_context_tool_calls() {
        let mut messages = serde_json::json!([
            {"role": "user", "content": "Help me"},
            {
                "role": "assistant",
                "content": "Let me check the context...",
                "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "aperture_context_preview", "arguments": "{}"}}
                ]
            },
            {"role": "tool", "tool_call_id": "call_1", "content": "Preview data..."},
            {"role": "assistant", "content": "Based on what I see..."}
        ]);

        let result = strip_openai_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = messages.as_array().unwrap();
        assert_eq!(msgs.len(), 3); // user, assistant (no more tool_calls), assistant
                                   // First assistant should keep content but lose tool_calls
        assert!(msgs[1].get("tool_calls").is_none());
        assert_eq!(msgs[1]["content"], "Let me check the context...");
    }

    #[test]
    fn test_strip_openai_mcp_namespaced_tools_are_preserved() {
        let mut messages = serde_json::json!([
            {"role": "user", "content": "Help me"},
            {
                "role": "assistant",
                "content": "Let me check the context...",
                "tool_calls": [
                    {
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "mcp__aperture__aperture_context_preview", "arguments": "{}"}
                    }
                ]
            },
            {"role": "tool", "tool_call_id": "call_1", "content": "Preview data..."}
        ]);

        let result = strip_openai_context_tools(&mut messages);
        assert_eq!(
            result.tool_uses_stripped, 0,
            "MCP tools in the last assistant message are recent — must be preserved"
        );
        assert_eq!(result.tool_results_stripped, 0);
        let msgs = messages.as_array().unwrap();
        // All 3 messages preserved — model's most recent tool cycle
        assert_eq!(msgs.len(), 3);
        assert!(msgs[1].get("tool_calls").is_some());
        assert_eq!(msgs[2]["tool_call_id"], "call_1");
    }

    /// Fix 3: Stale MCP context tools in OpenAI Chat format are stripped.
    #[test]
    fn test_strip_openai_stale_mcp_tools_stripped() {
        let mut messages = serde_json::json!([
            {"role": "user", "content": "Check context"},
            // Turn 1: model calls MCP preview (stale)
            {
                "role": "assistant",
                "content": "Checking...",
                "tool_calls": [
                    {"id": "call_old", "type": "function", "function": {"name": "mcp__aperture__aperture_context_preview", "arguments": "{}"}}
                ]
            },
            {"role": "tool", "tool_call_id": "call_old", "content": "Preview data..."},
            // Turn 2: model processed result, calls plan (recent)
            {
                "role": "assistant",
                "content": "I see the context. Let me plan...",
                "tool_calls": [
                    {"id": "call_recent", "type": "function", "function": {"name": "mcp__aperture__aperture_context_plan", "arguments": "{\"archive\":[\"1\"]}"}}
                ]
            },
            {"role": "tool", "tool_call_id": "call_recent", "content": "Plan staged."}
        ]);

        let result = strip_openai_context_tools(&mut messages);

        assert_eq!(result.tool_uses_stripped, 1, "stale MCP tool call should be stripped");
        assert_eq!(result.tool_results_stripped, 1, "stale MCP tool result should be stripped");

        let msgs = messages.as_array().unwrap();
        // user, assistant (text, no tool_calls), assistant (text + recent tool_call), tool (recent)
        assert_eq!(msgs.len(), 4);
        // First assistant: kept text, lost tool_calls (only had stale MCP call)
        assert!(msgs[1].get("tool_calls").is_none());
        assert_eq!(msgs[1]["content"], "Checking...");
        // Last assistant: kept both text and recent tool_call
        assert!(msgs[2].get("tool_calls").is_some());
        assert_eq!(msgs[2]["tool_calls"][0]["id"], "call_recent");
        // Recent tool result preserved
        assert_eq!(msgs[3]["tool_call_id"], "call_recent");
    }

    #[test]
    fn test_strip_openai_mixed_real_and_context_tools() {
        let mut messages = serde_json::json!([
            {
                "role": "assistant",
                "content": "Checking...",
                "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "aperture_context_preview", "arguments": "{}"}},
                    {"id": "call_2", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"foo.rs\"}"}}
                ]
            },
            {"role": "tool", "tool_call_id": "call_1", "content": "Context preview..."},
            {"role": "tool", "tool_call_id": "call_2", "content": "File contents..."}
        ]);

        let result = strip_openai_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = messages.as_array().unwrap();
        assert_eq!(msgs.len(), 2); // assistant + real tool result

        // Assistant should keep real tool call
        let tool_calls = msgs[0]["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["function"]["name"], "read_file");

        // Only real tool result remains
        assert_eq!(msgs[1]["tool_call_id"], "call_2");
    }

    #[test]
    fn test_strip_openai_only_context_tools_removes_assistant() {
        let mut messages = serde_json::json!([
            {"role": "user", "content": "Hello"},
            {
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "aperture_context_status", "arguments": "{}"}}
                ]
            },
            {"role": "tool", "tool_call_id": "call_1", "content": "Status..."}
        ]);

        let result = strip_openai_context_tools(&mut messages);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = messages.as_array().unwrap();
        // Assistant with null content and no tool_calls should be removed
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    // ── OpenAI Responses API Cleanup Tests ───────────────────

    #[test]
    fn test_strip_openai_responses_context_tools() {
        let mut input = serde_json::json!([
            {"type": "message", "role": "user", "content": "Help me"},
            {"type": "function_call", "name": "aperture_context_preview", "call_id": "fc_1", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "fc_1", "output": "Preview..."},
            {"type": "function_call", "name": "read_file", "call_id": "fc_2", "arguments": "{\"path\":\"foo.rs\"}"},
            {"type": "function_call_output", "call_id": "fc_2", "output": "File..."}
        ]);

        let result = strip_openai_responses_context_tools(&mut input);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let items = input.as_array().unwrap();
        assert_eq!(items.len(), 3); // user message + real function_call + real output
        assert_eq!(items[1]["name"], "read_file");
    }

    #[test]
    fn test_strip_openai_responses_mcp_namespaced_tools_are_preserved() {
        let mut input = serde_json::json!([
            {"type": "message", "role": "user", "content": "Help me"},
            {
                "type": "function_call",
                "name": "mcp__aperture__aperture_context_preview",
                "call_id": "fc_1",
                "arguments": "{}"
            },
            {"type": "function_call_output", "call_id": "fc_1", "output": "Preview..."}
        ]);

        let result = strip_openai_responses_context_tools(&mut input);
        assert_eq!(
            result.tool_uses_stripped, 0,
            "MCP tools with no subsequent assistant message are recent — must be preserved"
        );
        assert_eq!(result.tool_results_stripped, 0);
        let items = input.as_array().unwrap();
        // All 3 items preserved — model's most recent tool cycle
        assert_eq!(items.len(), 3);
        assert_eq!(items[1]["name"], "mcp__aperture__aperture_context_preview");
    }

    /// Fix 3: Stale MCP context tools in Responses API format are stripped.
    #[test]
    fn test_strip_openai_responses_stale_mcp_tools_stripped() {
        let mut input = serde_json::json!([
            {"type": "message", "role": "user", "content": "Check context"},
            // Stale MCP preview (model processes it in the assistant message below)
            {"type": "function_call", "name": "mcp__aperture__aperture_context_preview", "call_id": "fc_old", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "fc_old", "output": "Preview data..."},
            // Model processed result and responded
            {"type": "message", "role": "assistant", "content": "I see the context. Let me plan..."},
            // Recent MCP plan (after the assistant message — not yet processed)
            {"type": "function_call", "name": "mcp__aperture__aperture_context_plan", "call_id": "fc_recent", "arguments": "{\"archive\":[\"1\"]}"},
            {"type": "function_call_output", "call_id": "fc_recent", "output": "Plan staged."}
        ]);

        let result = strip_openai_responses_context_tools(&mut input);

        assert_eq!(result.tool_uses_stripped, 1, "stale MCP function_call should be stripped");
        assert_eq!(result.tool_results_stripped, 1, "stale MCP function_call_output should be stripped");

        let items = input.as_array().unwrap();
        // user message, assistant message, recent function_call, recent output
        assert_eq!(items.len(), 4);
        assert_eq!(items[0]["role"], "user");
        assert_eq!(items[1]["role"], "assistant");
        assert_eq!(items[2]["name"], "mcp__aperture__aperture_context_plan");
        assert_eq!(items[3]["call_id"], "fc_recent");
    }

    // ── Manifest Injection Tests ─────────────────────────────

    #[test]
    fn test_inject_manifest_anthropic_string_system() {
        let mut req = serde_json::json!({
            "model": "claude-3",
            "system": "You are helpful.",
            "messages": []
        });
        inject_manifest_anthropic(&mut req, "[Aperture: 45% | 12 blocks]");
        let system = req["system"].as_str().unwrap();
        assert!(system.starts_with("[Aperture: 45% | 12 blocks]"));
        assert!(system.contains("You are helpful."));
    }

    #[test]
    fn test_inject_manifest_anthropic_array_system() {
        let mut req = serde_json::json!({
            "model": "claude-3",
            "system": [
                {"type": "text", "text": "You are helpful."}
            ],
            "messages": []
        });
        inject_manifest_anthropic(&mut req, "[Aperture: 45% | 12 blocks]");
        let system = req["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["text"], "[Aperture: 45% | 12 blocks]");
    }

    #[test]
    fn test_inject_manifest_anthropic_no_system() {
        let mut req = serde_json::json!({
            "model": "claude-3",
            "messages": []
        });
        inject_manifest_anthropic(&mut req, "[Aperture: 50% | 8 blocks]");
        assert_eq!(req["system"], "[Aperture: 50% | 8 blocks]");
    }

    #[test]
    fn test_inject_manifest_openai_existing_system() {
        let mut messages = serde_json::json!([
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "Hello"}
        ]);
        inject_manifest_openai(&mut messages, "[Aperture: 45% | 12 blocks]");
        let system_content = messages[0]["content"].as_str().unwrap();
        assert!(system_content.starts_with("[Aperture: 45% | 12 blocks]"));
        assert!(system_content.contains("You are helpful."));
    }

    #[test]
    fn test_inject_manifest_openai_no_system() {
        let mut messages = serde_json::json!([
            {"role": "user", "content": "Hello"}
        ]);
        inject_manifest_openai(&mut messages, "[Aperture: 50% | 8 blocks]");
        let msgs = messages.as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "[Aperture: 50% | 8 blocks]");
    }

    #[test]
    fn test_inject_manifest_empty_is_noop() {
        let mut req = serde_json::json!({
            "system": "Original.",
            "messages": []
        });
        inject_manifest_anthropic(&mut req, "");
        assert_eq!(req["system"], "Original.");
    }
}
