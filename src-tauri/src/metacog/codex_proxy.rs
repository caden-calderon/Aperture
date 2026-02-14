//! Codex proxy runtime — tools injected into OpenAI API requests.
//!
//! For Codex CLI and other OpenAI-compatible clients. The proxy:
//! 1. Injects `aperture_context_*` tool definitions into the `tools[]` array
//! 2. Intercepts context tool calls from model responses
//! 3. Strips context tool calls/results from conversation history between turns
//! 4. Injects manifest into system message

use serde_json::Value;

use crate::engine::planner::cleanup::{
    inject_manifest_openai, strip_openai_context_tools, strip_openai_responses_context_tools,
};

use super::runtime::{
    context_tool_definitions, is_context_tool_name, CleanupResult, ContextToolCall, ContextToolDef,
    ContextToolResult, ContextToolRuntime, RuntimeKind,
};

/// Whether this is a Responses API or Chat Completions request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiFormat {
    /// `/v1/responses` — uses `input[]` and `tools[]` with `type: "function"`
    Responses,
    /// `/v1/chat/completions` — uses `messages[]` and `tools[]` with `type: "function"`
    ChatCompletions,
}

/// Codex proxy runtime — injects tools into OpenAI API requests.
pub struct CodexProxyRuntime {
    pub format: OpenAiFormat,
}

impl CodexProxyRuntime {
    pub fn new(format: OpenAiFormat) -> Self {
        Self { format }
    }

    /// Detect the format from a request path.
    pub fn detect_format(path: &str) -> OpenAiFormat {
        if path.contains("/responses") {
            OpenAiFormat::Responses
        } else {
            OpenAiFormat::ChatCompletions
        }
    }
}

impl ContextToolRuntime for CodexProxyRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::CodexProxy
    }

    fn tool_definitions(&self) -> Vec<ContextToolDef> {
        context_tool_definitions()
    }

    fn inject_tools(&self, request_json: &mut Value) {
        let defs = context_tool_definitions();
        let tools_json: Vec<Value> = defs
            .into_iter()
            .map(|def| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name,
                        "description": def.description,
                        "parameters": def.parameters_schema
                    }
                })
            })
            .collect();

        // Append to existing tools[] or create it
        match request_json.get_mut("tools").and_then(|t| t.as_array_mut()) {
            Some(existing) => {
                existing.extend(tools_json);
            }
            None => {
                request_json["tools"] = Value::Array(tools_json);
            }
        }
    }

    fn extract_context_calls(&self, response_json: &Value) -> Vec<ContextToolCall> {
        let mut calls = Vec::new();

        match self.format {
            OpenAiFormat::ChatCompletions => {
                // Chat Completions: choices[].message.tool_calls[]
                if let Some(choices) = response_json.get("choices").and_then(|c| c.as_array()) {
                    for choice in choices {
                        let tool_calls = choice
                            .get("message")
                            .and_then(|m| m.get("tool_calls"))
                            .and_then(|tc| tc.as_array());
                        if let Some(tcs) = tool_calls {
                            for tc in tcs {
                                if let Some(call) = extract_openai_tool_call(tc) {
                                    if is_context_tool_name(&call.name) {
                                        calls.push(call);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            OpenAiFormat::Responses => {
                // Responses API: output[] with type "function_call"
                if let Some(output) = response_json.get("output").and_then(|o| o.as_array()) {
                    for item in output {
                        if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                            let name = item
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            if is_context_tool_name(&name) {
                                let id = item
                                    .get("call_id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let arguments_str = item
                                    .get("arguments")
                                    .and_then(|a| a.as_str())
                                    .unwrap_or("{}");
                                let arguments = serde_json::from_str(arguments_str)
                                    .unwrap_or_else(|_| serde_json::json!({}));
                                calls.push(ContextToolCall {
                                    id,
                                    name,
                                    arguments,
                                });
                            }
                        }
                    }
                }
            }
        }

        calls
    }

    fn inject_results(&self, request_json: &mut Value, results: &[ContextToolResult]) {
        if results.is_empty() {
            return;
        }

        match self.format {
            OpenAiFormat::ChatCompletions => {
                // Inject as role: "tool" messages into messages[]
                let messages = match request_json
                    .get_mut("messages")
                    .and_then(|m| m.as_array_mut())
                {
                    Some(arr) => arr,
                    None => return,
                };
                for result in results {
                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": result.tool_use_id,
                        "content": result.content
                    }));
                }
            }
            OpenAiFormat::Responses => {
                // Inject as function_call_output items into input[]
                let input = match request_json.get_mut("input").and_then(|i| i.as_array_mut()) {
                    Some(arr) => arr,
                    None => return,
                };
                for result in results {
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": result.tool_use_id,
                        "output": result.content
                    }));
                }
            }
        }
    }

    fn cleanup_history(&self, request_json: &mut Value) -> CleanupResult {
        match self.format {
            OpenAiFormat::ChatCompletions => match request_json.get_mut("messages") {
                Some(messages) => strip_openai_context_tools(messages),
                None => CleanupResult::default(),
            },
            OpenAiFormat::Responses => match request_json.get_mut("input") {
                Some(input) => strip_openai_responses_context_tools(input),
                None => CleanupResult::default(),
            },
        }
    }

    fn inject_manifest(&self, request_json: &mut Value, manifest: &str) {
        match self.format {
            OpenAiFormat::ChatCompletions => {
                if let Some(messages) = request_json.get_mut("messages") {
                    inject_manifest_openai(messages, manifest);
                }
            }
            OpenAiFormat::Responses => {
                // Responses API: inject into `instructions` field (system-level)
                match request_json.get("instructions") {
                    Some(Value::String(existing)) => {
                        let new_instructions = format!("{manifest}\n\n{existing}");
                        request_json["instructions"] = Value::String(new_instructions);
                    }
                    _ => {
                        // Also check input[] for system message
                        if let Some(input) = request_json.get_mut("input") {
                            inject_manifest_openai(input, manifest);
                        }
                    }
                }
            }
        }
    }
}

/// Extract a ContextToolCall from an OpenAI-format tool_call object.
fn extract_openai_tool_call(tc: &Value) -> Option<ContextToolCall> {
    let id = tc.get("id")?.as_str()?.to_string();
    let function = tc.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let arguments_str = function.get("arguments")?.as_str()?;
    let arguments = serde_json::from_str(arguments_str).unwrap_or_else(|_| serde_json::json!({}));
    Some(ContextToolCall {
        id,
        name,
        arguments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Format Detection ─────────────────────────────────────

    #[test]
    fn test_detect_format_responses() {
        assert_eq!(
            CodexProxyRuntime::detect_format("/v1/responses"),
            OpenAiFormat::Responses
        );
        assert_eq!(
            CodexProxyRuntime::detect_format("/responses"),
            OpenAiFormat::Responses
        );
    }

    #[test]
    fn test_detect_format_chat_completions() {
        assert_eq!(
            CodexProxyRuntime::detect_format("/v1/chat/completions"),
            OpenAiFormat::ChatCompletions
        );
    }

    // ── Tool Injection ───────────────────────────────────────

    #[test]
    fn test_inject_tools_creates_tools_array() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::ChatCompletions);
        let mut request = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        runtime.inject_tools(&mut request);

        let tools = request["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 5);
        assert_eq!(tools[0]["function"]["name"], "aperture_context_preview");
        assert_eq!(tools[0]["type"], "function");
    }

    #[test]
    fn test_inject_tools_appends_to_existing() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::ChatCompletions);
        let mut request = serde_json::json!({
            "model": "gpt-4",
            "tools": [
                {"type": "function", "function": {"name": "read_file", "parameters": {}}}
            ]
        });
        runtime.inject_tools(&mut request);

        let tools = request["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 6); // 1 existing + 5 context tools
        assert_eq!(tools[0]["function"]["name"], "read_file");
        assert_eq!(tools[1]["function"]["name"], "aperture_context_preview");
    }

    // ── Call Extraction ──────────────────────────────────────

    #[test]
    fn test_extract_chat_completions_context_calls() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::ChatCompletions);
        let response = serde_json::json!({
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
                                "arguments": "{\"path\": \"foo.rs\"}"
                            }
                        }
                    ]
                }
            }]
        });

        let calls = runtime.extract_context_calls(&response);
        assert_eq!(calls.len(), 1); // Only context tool
        assert_eq!(calls[0].name, "aperture_context_preview");
        assert_eq!(calls[0].id, "call_1");
    }

    #[test]
    fn test_extract_responses_api_context_calls() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::Responses);
        let response = serde_json::json!({
            "output": [
                {
                    "type": "function_call",
                    "name": "aperture_context_status",
                    "call_id": "fc_1",
                    "arguments": "{}"
                },
                {
                    "type": "function_call",
                    "name": "write_file",
                    "call_id": "fc_2",
                    "arguments": "{\"path\": \"foo.rs\"}"
                },
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "Done"}]
                }
            ]
        });

        let calls = runtime.extract_context_calls(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "aperture_context_status");
        assert_eq!(calls[0].id, "fc_1");
    }

    #[test]
    fn test_extract_no_context_calls() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::ChatCompletions);
        let response = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello!",
                    "tool_calls": []
                }
            }]
        });

        let calls = runtime.extract_context_calls(&response);
        assert!(calls.is_empty());
    }

    // ── Result Injection ─────────────────────────────────────

    #[test]
    fn test_inject_results_chat_completions() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::ChatCompletions);
        let mut request = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let results = vec![ContextToolResult {
            tool_use_id: "call_1".into(),
            content: "Preview data...".into(),
            is_error: false,
        }];

        runtime.inject_results(&mut request, &results);
        let messages = request["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
    }

    #[test]
    fn test_inject_results_responses_api() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::Responses);
        let mut request = serde_json::json!({
            "input": [
                {"type": "message", "role": "user", "content": "Hello"}
            ]
        });

        let results = vec![ContextToolResult {
            tool_use_id: "fc_1".into(),
            content: "Status data...".into(),
            is_error: false,
        }];

        runtime.inject_results(&mut request, &results);
        let input = request["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], "fc_1");
    }

    // ── Cleanup ──────────────────────────────────────────────

    #[test]
    fn test_cleanup_chat_completions() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::ChatCompletions);
        let mut request = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Help me"},
                {
                    "role": "assistant",
                    "content": "Let me check...",
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "aperture_context_preview", "arguments": "{}"}}
                    ]
                },
                {"role": "tool", "tool_call_id": "call_1", "content": "Preview..."},
                {"role": "assistant", "content": "I see the context."}
            ]
        });

        let result = runtime.cleanup_history(&mut request);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let msgs = request["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3); // user, assistant (cleaned), assistant
    }

    #[test]
    fn test_cleanup_responses_api() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::Responses);
        let mut request = serde_json::json!({
            "input": [
                {"type": "message", "role": "user", "content": "Help"},
                {"type": "function_call", "name": "aperture_context_preview", "call_id": "fc_1", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "fc_1", "output": "Preview..."},
                {"type": "function_call", "name": "read_file", "call_id": "fc_2", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "fc_2", "output": "File..."}
            ]
        });

        let result = runtime.cleanup_history(&mut request);
        assert_eq!(result.tool_uses_stripped, 1);
        assert_eq!(result.tool_results_stripped, 1);

        let input = request["input"].as_array().unwrap();
        assert_eq!(input.len(), 3); // user + real function_call + output
    }

    // ── Manifest Injection ───────────────────────────────────

    #[test]
    fn test_inject_manifest_chat_completions() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::ChatCompletions);
        let mut request = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ]
        });
        runtime.inject_manifest(&mut request, "[Aperture: 45%]");
        let system = request["messages"][0]["content"].as_str().unwrap();
        assert!(system.starts_with("[Aperture: 45%]"));
    }

    #[test]
    fn test_inject_manifest_responses_api_with_instructions() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::Responses);
        let mut request = serde_json::json!({
            "instructions": "You are helpful.",
            "input": [{"type": "message", "role": "user", "content": "Hello"}]
        });
        runtime.inject_manifest(&mut request, "[Aperture: 50%]");
        let instructions = request["instructions"].as_str().unwrap();
        assert!(instructions.starts_with("[Aperture: 50%]"));
        assert!(instructions.contains("You are helpful."));
    }

    #[test]
    fn test_inject_manifest_responses_api_fallback_to_input() {
        let runtime = CodexProxyRuntime::new(OpenAiFormat::Responses);
        let mut request = serde_json::json!({
            "input": [
                {"role": "system", "content": "System prompt."},
                {"role": "user", "content": "Hello"}
            ]
        });
        runtime.inject_manifest(&mut request, "[Aperture: 55%]");
        let system = request["input"][0]["content"].as_str().unwrap();
        assert!(system.starts_with("[Aperture: 55%]"));
    }
}
