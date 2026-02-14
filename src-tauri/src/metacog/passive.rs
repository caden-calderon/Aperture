//! Passive runtime — manifest only, no tool surface.
//!
//! For clients that support neither MCP nor proxy tool injection.
//! The model receives the manifest (awareness) but cannot call context
//! management tools. All context management is handled by autonomous heuristics.

use serde_json::Value;

use crate::engine::planner::cleanup::{inject_manifest_anthropic, inject_manifest_openai};

use super::runtime::{
    CleanupResult, ContextToolCall, ContextToolDef, ContextToolResult, ContextToolRuntime,
    RuntimeKind,
};

/// Passive runtime — manifest injection only, no tool surface.
pub struct PassiveRuntime {
    /// Whether this session uses Anthropic message format.
    /// Determines how manifest is injected into system message.
    pub is_anthropic: bool,
}

impl PassiveRuntime {
    pub fn new(is_anthropic: bool) -> Self {
        Self { is_anthropic }
    }
}

impl ContextToolRuntime for PassiveRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Passive
    }

    fn tool_definitions(&self) -> Vec<ContextToolDef> {
        vec![] // No tools exposed
    }

    fn inject_tools(&self, _request_json: &mut Value) {
        // No-op — passive runtime has no tool surface
    }

    fn extract_context_calls(&self, _response_json: &Value) -> Vec<ContextToolCall> {
        vec![] // No tools to call
    }

    fn inject_results(&self, _request_json: &mut Value, _results: &[ContextToolResult]) {
        // No-op — no tools means no results to inject
    }

    fn cleanup_history(&self, _request_json: &mut Value) -> CleanupResult {
        // No-op — no context tools to clean up
        CleanupResult::default()
    }

    fn inject_manifest(&self, request_json: &mut Value, manifest: &str) {
        if self.is_anthropic {
            inject_manifest_anthropic(request_json, manifest);
        } else {
            // OpenAI: inject into the messages/input array
            let key = if request_json.get("input").is_some() {
                "input"
            } else {
                "messages"
            };
            if let Some(messages) = request_json.get_mut(key) {
                inject_manifest_openai(messages, manifest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passive_kind() {
        let runtime = PassiveRuntime::new(false);
        assert_eq!(runtime.kind(), RuntimeKind::Passive);
    }

    #[test]
    fn test_passive_no_tools() {
        let runtime = PassiveRuntime::new(false);
        assert!(runtime.tool_definitions().is_empty());
    }

    #[test]
    fn test_passive_no_context_calls_extracted() {
        let runtime = PassiveRuntime::new(false);
        let response = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "Hi"}}]
        });
        assert!(runtime.extract_context_calls(&response).is_empty());
    }

    #[test]
    fn test_passive_cleanup_is_noop() {
        let runtime = PassiveRuntime::new(false);
        let mut request = serde_json::json!({"messages": [{"role": "user", "content": "Hello"}]});
        let result = runtime.cleanup_history(&mut request);
        assert_eq!(result.tool_uses_stripped, 0);
    }

    #[test]
    fn test_passive_inject_manifest_anthropic() {
        let runtime = PassiveRuntime::new(true);
        let mut request = serde_json::json!({
            "model": "claude-3",
            "system": "You are helpful.",
            "messages": []
        });
        runtime.inject_manifest(&mut request, "[Aperture: 45% | 12 blocks]");
        let system = request["system"].as_str().unwrap();
        assert!(system.starts_with("[Aperture: 45% | 12 blocks]"));
    }

    #[test]
    fn test_passive_inject_manifest_openai_messages() {
        let runtime = PassiveRuntime::new(false);
        let mut request = serde_json::json!({
            "model": "gpt-4",
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
    fn test_passive_inject_manifest_openai_input() {
        let runtime = PassiveRuntime::new(false);
        let mut request = serde_json::json!({
            "model": "gpt-4",
            "input": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ]
        });
        runtime.inject_manifest(&mut request, "[Aperture: 50%]");
        let system = request["input"][0]["content"].as_str().unwrap();
        assert!(system.starts_with("[Aperture: 50%]"));
    }
}
