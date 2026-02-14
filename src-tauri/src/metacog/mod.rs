//! Metacognition module — context awareness tools for LLM models.
//!
//! Provides structured context management tools through client-specific
//! runtime adapters. The shared core (planner, tools, preview) is identical
//! across all runtimes; thin adapters handle transport differences.
//!
//! ## Runtimes
//! - `ClaudeMcpRuntime` — MCP server for Claude Code (Anthropic format)
//! - `CodexProxyRuntime` — Proxy-injected tools for Codex CLI (OpenAI format)
//! - `PassiveRuntime` — Manifest only, no tool surface (any format)

pub mod claude_mcp;
pub mod codex_proxy;
pub mod passive;
pub mod preview;
pub mod runtime;
pub mod tools;

// Re-export key types for convenience
pub use runtime::{
    context_tool_definitions, detect_runtime, is_context_tool_name, CleanupResult, ContextToolCall,
    ContextToolDef, ContextToolResult, ContextToolRuntime, RuntimeKind, CONTEXT_TOOL_PREFIX,
};
pub use tools::{dispatch_tool, ToolOutput};

pub use claude_mcp::ClaudeMcpRuntime;
pub use codex_proxy::{CodexProxyRuntime, OpenAiFormat};
pub use passive::PassiveRuntime;

/// Create the appropriate runtime adapter for a given kind and request path.
pub fn select_runtime(kind: RuntimeKind, path: &str) -> Box<dyn ContextToolRuntime> {
    match kind {
        RuntimeKind::ClaudeMcp => Box::new(ClaudeMcpRuntime::new()),
        RuntimeKind::CodexProxy => {
            let format = CodexProxyRuntime::detect_format(path);
            Box::new(CodexProxyRuntime::new(format))
        }
        RuntimeKind::Passive => {
            // Detect whether this is an Anthropic-format request
            let is_anthropic = path.contains("/messages");
            Box::new(PassiveRuntime::new(is_anthropic))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_runtime_claude() {
        let rt = select_runtime(RuntimeKind::ClaudeMcp, "/v1/messages");
        assert_eq!(rt.kind(), RuntimeKind::ClaudeMcp);
        assert_eq!(rt.tool_definitions().len(), 5);
    }

    #[test]
    fn test_select_runtime_codex_responses() {
        let rt = select_runtime(RuntimeKind::CodexProxy, "/v1/responses");
        assert_eq!(rt.kind(), RuntimeKind::CodexProxy);
        assert_eq!(rt.tool_definitions().len(), 5);
    }

    #[test]
    fn test_select_runtime_codex_chat() {
        let rt = select_runtime(RuntimeKind::CodexProxy, "/v1/chat/completions");
        assert_eq!(rt.kind(), RuntimeKind::CodexProxy);
    }

    #[test]
    fn test_select_runtime_passive() {
        let rt = select_runtime(RuntimeKind::Passive, "/unknown");
        assert_eq!(rt.kind(), RuntimeKind::Passive);
        assert!(rt.tool_definitions().is_empty());
    }

    #[test]
    fn test_select_runtime_passive_anthropic() {
        let rt = select_runtime(RuntimeKind::Passive, "/v1/messages");
        assert_eq!(rt.kind(), RuntimeKind::Passive);
    }
}
