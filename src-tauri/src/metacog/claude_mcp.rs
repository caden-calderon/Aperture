//! Claude MCP runtime — Anthropic message format adapter.
//!
//! For Claude Code via MCP server. Tools are registered through the MCP
//! protocol (not injected into API requests). This adapter handles:
//! - Extracting context tool calls from Anthropic-format responses
//! - Injecting tool results into Anthropic-format requests
//! - Cleaning up ephemeral context tool calls from conversation history
//! - Injecting manifests into the `system` field
//!
//! Note: The actual MCP server transport (stdio process) is a separate concern.
//! This adapter handles message format transformation only.

use serde_json::Value;

use crate::engine::planner::cleanup::{inject_manifest_anthropic, strip_anthropic_context_tools};

use super::runtime::{
    context_tool_definitions, is_context_tool_name, CleanupResult, ContextToolCall, ContextToolDef,
    ContextToolResult, ContextToolRuntime, RuntimeKind,
};

/// Claude MCP runtime — Anthropic message format, MCP tool registration.
pub struct ClaudeMcpRuntime;

impl ClaudeMcpRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeMcpRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextToolRuntime for ClaudeMcpRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ClaudeMcp
    }

    fn tool_definitions(&self) -> Vec<ContextToolDef> {
        context_tool_definitions()
    }

    fn inject_tools(&self, _request_json: &mut Value) {
        // No-op — Claude Code discovers tools via MCP, not API request injection.
        // The MCP server exposes tools through the MCP protocol handshake.
    }

    fn extract_context_calls(&self, response_json: &Value) -> Vec<ContextToolCall> {
        let mut calls = Vec::new();

        // Anthropic response format: content[] array with tool_use blocks
        let content = match response_json.get("content").and_then(|c| c.as_array()) {
            Some(arr) => arr,
            None => return calls,
        };

        for block in content {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            let name = match block.get("name").and_then(|n| n.as_str()) {
                Some(n) if is_context_tool_name(n) => n.to_string(),
                _ => continue,
            };
            let id = block
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            let arguments = block
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));

            calls.push(ContextToolCall {
                id,
                name,
                arguments,
            });
        }

        calls
    }

    fn inject_results(&self, request_json: &mut Value, results: &[ContextToolResult]) {
        if results.is_empty() {
            return;
        }

        // Anthropic format: tool results go as a user message with tool_result content blocks.
        let messages = match request_json
            .get_mut("messages")
            .and_then(|m| m.as_array_mut())
        {
            Some(arr) => arr,
            None => return,
        };

        let result_blocks: Vec<Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": r.tool_use_id,
                    "content": r.content,
                    "is_error": r.is_error
                })
            })
            .collect();

        messages.push(serde_json::json!({
            "role": "user",
            "content": result_blocks
        }));
    }

    fn cleanup_history(&self, request_json: &mut Value) -> CleanupResult {
        match request_json.get_mut("messages") {
            Some(messages) => strip_anthropic_context_tools(messages),
            None => CleanupResult::default(),
        }
    }

    fn inject_manifest(&self, request_json: &mut Value, manifest: &str) {
        inject_manifest_anthropic(request_json, manifest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_mcp_kind() {
        let runtime = ClaudeMcpRuntime::new();
        assert_eq!(runtime.kind(), RuntimeKind::ClaudeMcp);
    }

    #[test]
    fn test_claude_mcp_tool_definitions() {
        let runtime = ClaudeMcpRuntime::new();
        let defs = runtime.tool_definitions();
        assert_eq!(defs.len(), 5);
    }

    #[test]
    fn test_claude_mcp_inject_tools_is_noop() {
        let runtime = ClaudeMcpRuntime::new();
        let mut request = serde_json::json!({
            "model": "claude-3",
            "messages": []
        });
        let original = request.clone();
        runtime.inject_tools(&mut request);
        assert_eq!(request, original);
    }

    #[test]
    fn test_claude_mcp_extract_context_calls() {
        let runtime = ClaudeMcpRuntime::new();
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "Let me check the context..."},
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "aperture_context_preview",
                    "input": {}
                },
                {
                    "type": "tool_use",
                    "id": "toolu_2",
                    "name": "read_file",
                    "input": {"path": "foo.rs"}
                }
            ]
        });

        let calls = runtime.extract_context_calls(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "aperture_context_preview");
        assert_eq!(calls[0].id, "toolu_1");
    }

    #[test]
    fn test_claude_mcp_extract_no_context_calls() {
        let runtime = ClaudeMcpRuntime::new();
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "Hello!"},
                {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {"path": "foo.rs"}}
            ]
        });

        let calls = runtime.extract_context_calls(&response);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_claude_mcp_inject_results() {
        let runtime = ClaudeMcpRuntime::new();
        let mut request = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "aperture_context_preview", "input": {}}
                ]}
            ]
        });

        let results = vec![ContextToolResult {
            tool_use_id: "toolu_1".into(),
            content: "Preview data here".into(),
            is_error: false,
        }];

        runtime.inject_results(&mut request, &results);
        let messages = request["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2]["role"], "user");

        let content = messages[2]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "toolu_1");
        assert_eq!(content[0]["content"], "Preview data here");
    }

    #[test]
    fn test_claude_mcp_cleanup() {
        let runtime = ClaudeMcpRuntime::new();
        let mut request = serde_json::json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "Help"}]},
                {
                    "role": "assistant",
                    "content": [
                        {"type": "text", "text": "Checking..."},
                        {"type": "tool_use", "id": "toolu_1", "name": "aperture_context_preview", "input": {}}
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "Preview..."}
                    ]
                },
                {"role": "assistant", "content": [{"type": "text", "text": "I see."}]}
            ]
        });

        let result = runtime.cleanup_history(&mut request);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = request["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3); // user, assistant (text only), assistant
    }

    #[test]
    fn test_claude_mcp_inject_manifest() {
        let runtime = ClaudeMcpRuntime::new();
        let mut request = serde_json::json!({
            "system": "You are helpful.",
            "messages": []
        });
        runtime.inject_manifest(&mut request, "[Aperture: 45% | 12 blocks]");
        let system = request["system"].as_str().unwrap();
        assert!(system.starts_with("[Aperture: 45% | 12 blocks]"));
        assert!(system.contains("You are helpful."));
    }
}
