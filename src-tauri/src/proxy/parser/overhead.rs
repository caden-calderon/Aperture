/// Estimate token overhead from tools + system message in a raw request JSON.
///
/// Uses bytes/4 as a rough approximation — close enough for JSON tool schemas
/// and system prompt text. Both contribute to the LLM's context window but are
/// not represented as conversation blocks.
///
/// System message extraction:
/// - Anthropic: `raw["system"]` — string or content-block array
/// - OpenAI Responses: `raw["instructions"]` — string
/// - OpenAI Chat: `raw["messages"][0]` where `role == "system"` — string or content parts
pub(super) fn estimate_request_overhead(raw: &serde_json::Value) -> u32 {
    let tool_tokens = raw
        .get("tools")
        .filter(|v| v.is_array())
        .and_then(|tools| serde_json::to_string(tools).ok())
        .map(|s| (s.len() as u32) / 4)
        .unwrap_or(0);

    let system_bytes = estimate_system_message_bytes(raw);
    let system_tokens = (system_bytes as u32) / 4;

    tool_tokens + system_tokens
}

/// Extract the approximate byte length of the system message from a raw request.
fn estimate_system_message_bytes(raw: &serde_json::Value) -> usize {
    // Anthropic: top-level "system" field (string or content-block array)
    if let Some(system) = raw.get("system") {
        return match system {
            serde_json::Value::String(s) => s.len(),
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .map(|t| t.len())
                .sum(),
            _ => 0,
        };
    }

    // OpenAI Responses: top-level "instructions" field
    if let Some(instructions) = raw.get("instructions").and_then(|v| v.as_str()) {
        return instructions.len();
    }

    // OpenAI Chat Completions: messages[0] with role "system"
    if let Some(messages) = raw.get("messages").and_then(|v| v.as_array()) {
        if let Some(first) = messages.first() {
            if first.get("role").and_then(|r| r.as_str()) == Some("system") {
                if let Some(content) = first.get("content") {
                    return match content {
                        serde_json::Value::String(s) => s.len(),
                        serde_json::Value::Array(parts) => parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                            .map(|t| t.len())
                            .sum(),
                        _ => 0,
                    };
                }
            }
        }
    }

    0
}
