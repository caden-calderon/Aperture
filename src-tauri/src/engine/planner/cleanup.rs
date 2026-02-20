//! Ephemeral cleanup system — strips context tool calls and generates breadcrumbs.
//!
//! Between turns, all `aperture_context_*` tool_use and tool_result entries are
//! stripped from conversation history and replaced with a breadcrumb summary.
//! This keeps the model's history clean while preserving awareness of what changed.
//!
//! Supports both Anthropic and OpenAI message formats.

use serde_json::Value;

use crate::metacog::runtime::{is_intercepted_context_tool_name, CleanupResult};

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

    // First pass: collect interceptor context tool_use IDs from assistant messages.
    // Only strip bare-prefix tools (proxy-injected). MCP-namespaced tools
    // (mcp__aperture__*) are legitimate conversation entries and must be preserved
    // so the model remembers calling them.
    let mut context_tool_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for msg in msgs.iter() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let content = match msg.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                    if is_intercepted_context_tool_name(name) {
                        if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                            context_tool_ids.insert(id.to_string());
                        }
                    }
                }
            }
        }
    }

    if context_tool_ids.is_empty() {
        return CleanupResult::default();
    }

    let mut tool_uses_stripped = 0usize;
    let mut tool_results_stripped = 0usize;

    // Second pass: strip interceptor context tool_use blocks from assistant messages
    // and context tool_result blocks from user messages.
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
                        if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                            if is_intercepted_context_tool_name(name) {
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
                            if context_tool_ids.contains(id) {
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

    // First pass: collect interceptor context tool_call IDs from assistant messages.
    // Only strip bare-prefix tools (proxy-injected). MCP-namespaced tools are preserved.
    let mut context_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for msg in msgs.iter() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "assistant" {
            continue;
        }
        let tool_calls = match msg.get("tool_calls").and_then(|tc| tc.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        for tc in tool_calls {
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            if is_intercepted_context_tool_name(name) {
                if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                    context_call_ids.insert(id.to_string());
                }
            }
        }
    }

    if context_call_ids.is_empty() {
        return CleanupResult::default();
    }

    let mut tool_uses_stripped = 0usize;
    let mut tool_results_stripped = 0usize;

    // Second pass: strip interceptor context tool calls from assistant messages,
    // and remove tool result messages entirely if they match.
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
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    !is_intercepted_context_tool_name(name)
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

    // Third pass: remove tool result messages that match context call IDs.
    msgs.retain(|msg| {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "tool" {
            if let Some(id) = msg.get("tool_call_id").and_then(|i| i.as_str()) {
                if context_call_ids.contains(id) {
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

    // Collect interceptor context function call IDs.
    // Only strip bare-prefix tools (proxy-injected). MCP-namespaced tools are preserved.
    let mut context_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for item in items.iter() {
        if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                if is_intercepted_context_tool_name(name) {
                    if let Some(id) = item.get("call_id").and_then(|i| i.as_str()) {
                        context_call_ids.insert(id.to_string());
                    }
                }
            }
        }
    }

    if context_call_ids.is_empty() {
        return CleanupResult::default();
    }

    let mut tool_uses_stripped = 0usize;
    let mut tool_results_stripped = 0usize;

    items.retain(|item| {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match item_type {
            "function_call" => {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    if is_intercepted_context_tool_name(name) {
                        tool_uses_stripped += 1;
                        return false;
                    }
                }
                true
            }
            "function_call_output" => {
                if let Some(id) = item.get("call_id").and_then(|i| i.as_str()) {
                    if context_call_ids.contains(id) {
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
            "MCP-namespaced tool names must NOT be stripped (model needs memory)"
        );
        assert_eq!(result.tool_results_stripped, 0);
        // Both messages preserved — model remembers calling the tool
        let msgs = messages.as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        let assistant_content = msgs[0]["content"].as_array().unwrap();
        assert_eq!(assistant_content.len(), 2);
        assert_eq!(assistant_content[1]["name"], "mcp__aperture__aperture_context_preview");
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
            "MCP-namespaced tool names must NOT be stripped (model needs memory)"
        );
        assert_eq!(result.tool_results_stripped, 0);
        let msgs = messages.as_array().unwrap();
        // All 3 messages preserved — model remembers calling the tool
        assert_eq!(msgs.len(), 3);
        assert!(msgs[1].get("tool_calls").is_some());
        assert_eq!(msgs[2]["tool_call_id"], "call_1");
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
            "MCP-namespaced tool names must NOT be stripped (model needs memory)"
        );
        assert_eq!(result.tool_results_stripped, 0);
        let items = input.as_array().unwrap();
        // All 3 items preserved — model remembers calling the tool
        assert_eq!(items.len(), 3);
        assert_eq!(items[1]["name"], "mcp__aperture__aperture_context_preview");
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
