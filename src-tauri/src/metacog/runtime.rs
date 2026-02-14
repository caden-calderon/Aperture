//! Context tool runtime trait and shared types.
//!
//! The `ContextToolRuntime` trait defines the interface for exposing context
//! management tools to different LLM clients. Three adapters implement this:
//! - `ClaudeMcpRuntime` — MCP server for Claude Code
//! - `CodexProxyRuntime` — Proxy-injected tools for Codex CLI
//! - `PassiveRuntime` — Manifest only, no tool surface

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Definition of a context tool exposed to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextToolDef {
    /// Tool name (e.g. "aperture_context_preview").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters_schema: serde_json::Value,
}

/// A context tool call extracted from a model response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextToolCall {
    /// Unique ID for this tool use (for matching results).
    pub id: String,
    /// Tool name that was called.
    pub name: String,
    /// Arguments passed to the tool.
    pub arguments: serde_json::Value,
}

/// Result of executing a context tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextToolResult {
    /// ID matching the original tool call.
    pub tool_use_id: String,
    /// Result content (typically JSON string).
    pub content: String,
    /// Whether the tool execution succeeded.
    pub is_error: bool,
}

/// Result of cleanup operation (stripping ephemeral tool calls).
#[derive(Debug, Clone, Default)]
pub struct CleanupResult {
    /// Number of tool_use entries stripped.
    pub tool_uses_stripped: usize,
    /// Number of tool_result entries stripped.
    pub tool_results_stripped: usize,
    /// Breadcrumb text inserted in place of stripped calls.
    pub breadcrumb: Option<String>,
}

/// Prefix for all Aperture context tool names.
pub const CONTEXT_TOOL_PREFIX: &str = "aperture_context_";

/// Check whether a tool name is an Aperture context management tool.
pub fn is_context_tool_name(name: &str) -> bool {
    name.starts_with(CONTEXT_TOOL_PREFIX)
}

/// The adapter interface for exposing context management tools to LLM clients.
///
/// Each runtime handles provider-specific JSON format differences:
/// - How tool definitions are registered/injected
/// - How tool calls are extracted from responses
/// - How tool results are injected into requests
/// - How ephemeral tool calls are cleaned from history
/// - How manifests are injected into system messages
pub trait ContextToolRuntime: Send + Sync {
    /// Runtime kind identifier.
    fn kind(&self) -> RuntimeKind;

    /// Tool definitions to expose to this client.
    /// Returns empty vec for passive runtime.
    fn tool_definitions(&self) -> Vec<ContextToolDef>;

    /// Inject tool definitions into an outgoing API request.
    ///
    /// - MCP: no-op (tools registered via MCP protocol, not in API request)
    /// - Codex: injects into `tools[]` array
    /// - Passive: no-op
    fn inject_tools(&self, request_json: &mut Value);

    /// Extract context tool calls from a model response.
    ///
    /// Only returns calls to `aperture_context_*` tools.
    /// Real tool calls are ignored (left for the client to handle).
    fn extract_context_calls(&self, response_json: &Value) -> Vec<ContextToolCall>;

    /// Inject context tool results into the next request's messages.
    fn inject_results(&self, request_json: &mut Value, results: &[ContextToolResult]);

    /// Clean up ephemeral context tool calls from conversation history.
    ///
    /// Strips all `aperture_context_*` tool_use and matching tool_result entries
    /// from the messages array. Preserves all non-context tool calls.
    fn cleanup_history(&self, request_json: &mut Value) -> CleanupResult;

    /// Inject manifest text into the system message of a request.
    fn inject_manifest(&self, request_json: &mut Value, manifest: &str);
}

/// Which client runtime to use for this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Claude Code via MCP server.
    ClaudeMcp,
    /// Codex CLI via proxy-injected tools.
    CodexProxy,
    /// Unknown client — manifest only, no tool surface.
    Passive,
}

/// Detect the appropriate runtime from the API path and provider.
pub fn detect_runtime(path: &str, provider: &str) -> RuntimeKind {
    let path_lower = path.to_lowercase();
    let provider_lower = provider.to_lowercase();

    // Anthropic API paths → Claude MCP runtime
    if path_lower.contains("/v1/messages")
        || provider_lower.contains("anthropic")
        || provider_lower.contains("claude")
    {
        return RuntimeKind::ClaudeMcp;
    }

    // OpenAI API paths → Codex proxy runtime
    if path_lower.contains("/v1/responses")
        || path_lower.contains("/v1/chat/completions")
        || path_lower.contains("/responses")
        || provider_lower.contains("openai")
        || provider_lower.contains("codex")
    {
        return RuntimeKind::CodexProxy;
    }

    // Fallback
    RuntimeKind::Passive
}

/// Build the standard set of context tool definitions.
pub fn context_tool_definitions() -> Vec<ContextToolDef> {
    vec![
        ContextToolDef {
            name: "aperture_context_preview".into(),
            description: "Get a preview of all context blocks with smart-extracted summaries, grouped by zone. Use this to understand what's in your context window.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ContextToolDef {
            name: "aperture_context_read".into(),
            description: "Read the full content of a specific context block by ID.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "block_id": {
                        "type": "string",
                        "description": "The block ID to read"
                    }
                },
                "required": ["block_id"],
                "additionalProperties": false
            }),
        },
        ContextToolDef {
            name: "aperture_context_search".into(),
            description: "Search across active and optionally archived context blocks by keyword.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (matched against content, file paths, role)"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["active", "all"],
                        "description": "Search scope: 'active' (default) or 'all' (includes archived)"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ContextToolDef {
            name: "aperture_context_plan".into(),
            description: "Plan context changes: archive, compress, expand, pin, shift blocks. Changes are previewed and applied between turns. Last call wins.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expand": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Block IDs to expand to full content"
                    },
                    "archive": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Block IDs to remove from payload (kept in storage)"
                    },
                    "recall": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Archived block IDs to bring back into active context"
                    },
                    "pin": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Block IDs to pin (prevent auto-archival)"
                    },
                    "unpin": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Block IDs to unpin"
                    },
                    "shift_to": {
                        "type": "object",
                        "additionalProperties": {"type": "string", "enum": ["primacy", "middle", "recency"]},
                        "description": "Block ID → target zone"
                    },
                    "compress": {
                        "type": "object",
                        "additionalProperties": {"type": "string"},
                        "description": "Block ID → model-authored summary text"
                    },
                    "split": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "object",
                            "properties": {
                                "at": {"type": "integer"},
                                "archive_before": {"type": "boolean"}
                            },
                            "required": ["at"]
                        },
                        "description": "Thread ID → split instructions"
                    }
                },
                "additionalProperties": false
            }),
        },
        ContextToolDef {
            name: "aperture_context_status".into(),
            description: "Get a detailed manifest of your full context state: budget, blocks by zone, token counts.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_runtime_anthropic() {
        assert_eq!(
            detect_runtime("/v1/messages", "anthropic"),
            RuntimeKind::ClaudeMcp
        );
        assert_eq!(
            detect_runtime("/api/something", "claude"),
            RuntimeKind::ClaudeMcp
        );
    }

    #[test]
    fn test_detect_runtime_openai() {
        assert_eq!(
            detect_runtime("/v1/responses", "openai"),
            RuntimeKind::CodexProxy
        );
        assert_eq!(
            detect_runtime("/v1/chat/completions", "openai"),
            RuntimeKind::CodexProxy
        );
        assert_eq!(
            detect_runtime("/responses", "codex"),
            RuntimeKind::CodexProxy
        );
    }

    #[test]
    fn test_detect_runtime_passive() {
        assert_eq!(detect_runtime("/unknown", "unknown"), RuntimeKind::Passive);
    }

    #[test]
    fn test_tool_definitions_count() {
        let defs = context_tool_definitions();
        assert_eq!(defs.len(), 5);
        assert_eq!(defs[0].name, "aperture_context_preview");
        assert_eq!(defs[1].name, "aperture_context_read");
        assert_eq!(defs[2].name, "aperture_context_search");
        assert_eq!(defs[3].name, "aperture_context_plan");
        assert_eq!(defs[4].name, "aperture_context_status");
    }

    #[test]
    fn test_tool_schemas_are_valid_json() {
        for def in context_tool_definitions() {
            assert!(
                def.parameters_schema.is_object(),
                "Schema for {} should be an object",
                def.name
            );
        }
    }

    #[test]
    fn test_is_context_tool_name() {
        assert!(is_context_tool_name("aperture_context_preview"));
        assert!(is_context_tool_name("aperture_context_read"));
        assert!(is_context_tool_name("aperture_context_search"));
        assert!(is_context_tool_name("aperture_context_plan"));
        assert!(is_context_tool_name("aperture_context_status"));
        assert!(!is_context_tool_name("read_file"));
        assert!(!is_context_tool_name("aperture_other"));
        assert!(!is_context_tool_name(""));
    }
}
