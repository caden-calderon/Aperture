//! Request/response parsing for Anthropic and OpenAI API formats.
//!
//! Converts raw API JSON into universal `Block` structs that the context engine
//! and UI can work with. Handles:
//! - Anthropic Messages API (`/v1/messages`)
//! - OpenAI Chat Completions (`/v1/chat/completions`)
//! - OpenAI Responses API (`/v1/responses`)

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
use crate::engine::types::{CompressionLevel, Role, Zone};

// ============================================================================
// Provider detection
// ============================================================================

/// Detected API provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    OpenAI,
}

impl Provider {
    /// Detect provider from request path.
    ///
    /// Matches both `/v1/` prefixed and bare paths (e.g. Codex hits `/responses`
    /// without the `/v1/` prefix).
    pub fn from_path(path: &str) -> Option<Self> {
        if is_messages_path(path) {
            Some(Provider::Anthropic)
        } else if is_chat_completions_path(path) || is_responses_path(path) {
            Some(Provider::OpenAI)
        } else {
            None
        }
    }
}

/// Check if path targets the Anthropic Messages API.
pub(crate) fn is_messages_path(path: &str) -> bool {
    path == "/v1/messages"
        || path.starts_with("/v1/messages?")
        || path.starts_with("/v1/messages/")
        || path == "/messages"
        || path.starts_with("/messages?")
        || path.starts_with("/messages/")
}

/// Check if path targets OpenAI Chat Completions.
pub(crate) fn is_chat_completions_path(path: &str) -> bool {
    path == "/v1/chat/completions"
        || path.starts_with("/v1/chat/completions?")
        || path.starts_with("/v1/chat/completions/")
        || path == "/chat/completions"
        || path.starts_with("/chat/completions?")
        || path.starts_with("/chat/completions/")
}

/// Check if path targets OpenAI Responses API.
pub(crate) fn is_responses_path(path: &str) -> bool {
    path == "/v1/responses"
        || path.starts_with("/v1/responses?")
        || path.starts_with("/v1/responses/")
        || path == "/responses"
        || path.starts_with("/responses?")
        || path.starts_with("/responses/")
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Anthropic => write!(f, "anthropic"),
            Provider::OpenAI => write!(f, "openai"),
        }
    }
}

// ============================================================================
// Parsed output types
// ============================================================================

/// Token usage extracted from an API response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Result of parsing a request body.
#[derive(Debug, Clone)]
pub struct ParsedRequest {
    pub provider: Provider,
    pub model: String,
    pub blocks: Vec<Block>,
    /// System prompt extracted from request (Anthropic top-level `system` field).
    pub system_prompt: Option<String>,
    /// Whether the request asks for streaming (`"stream": true`).
    pub stream: bool,
}

/// Result of parsing a response body.
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    pub provider: Provider,
    pub blocks: Vec<Block>,
    pub usage: Option<TokenUsage>,
    pub model: Option<String>,
}

// ============================================================================
// Anthropic message format types (serde)
// ============================================================================

#[derive(Debug, Deserialize)]
struct AnthropicRequest {
    model: String,
    #[serde(default)]
    messages: Vec<AnthropicMessage>,
    system: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
    // tool_use fields
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    // tool_result fields
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    content: Option<serde_json::Value>,
    // thinking block fields
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    content: Vec<AnthropicResponseContent>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponseContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
    // tool_use
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    // thinking block
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

// ============================================================================
// OpenAI Chat Completions format types
// ============================================================================

#[derive(Debug, Deserialize)]
struct OpenAIChatRequest {
    model: String,
    #[serde(default)]
    messages: Vec<OpenAIChatMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatMessage {
    role: String,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolCall {
    id: String,
    function: OpenAIFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<OpenAIChatChoice>,
    #[serde(default)]
    usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatChoice {
    message: OpenAIChatMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

// ============================================================================
// OpenAI Responses API format types
// ============================================================================

#[derive(Debug, Deserialize)]
struct OpenAIResponsesRequest {
    model: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    instructions: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponsesResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    output: Vec<OpenAIResponsesOutput>,
    #[serde(default)]
    usage: Option<OpenAIResponsesUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponsesOutput {
    #[serde(rename = "type")]
    output_type: String,
    #[serde(default)]
    content: Option<Vec<OpenAIResponsesContent>>,
    // function_call output
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponsesContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponsesUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

// ============================================================================
// Block construction helper
// ============================================================================

fn now_iso8601() -> String {
    crate::util::iso_now()
}

/// Estimate token count from text content.
///
/// Uses a simple heuristic (chars / 4) as a fast approximation.
/// tiktoken-rs integration is available for precise counts in Phase 2.
fn estimate_tokens(content: &str) -> u32 {
    // Rough approximation: ~4 chars per token for English text
    (content.len() as f64 / 4.0).ceil() as u32
}

fn make_block(role: Role, content: String, provider: &str, turn_index: u32) -> Block {
    let tokens = estimate_tokens(&content);
    Block {
        id: Uuid::new_v4().to_string(),
        role,
        block_type: None,
        content: content.clone(),
        tokens,
        timestamp: now_iso8601(),
        zone: default_zone_for_role(role),
        pinned: None,
        compression_level: CompressionLevel::Original,
        compressed_versions: CompressionVersions {
            original: CompressionVersion { content, tokens },
            trimmed: None,
            summarized: None,
            minimal: None,
        },
        usage_heat: 0.0,
        position_relevance: 0.0,
        last_referenced_turn: turn_index,
        reference_count: 0,
        topic_cluster: None,
        topic_keywords: Vec::new(),
        metadata: BlockMetadata {
            provider: provider.to_string(),
            turn_index,
            tool_name: None,
            file_paths: Vec::new(),
        },
    }
}

fn make_tool_block(
    role: Role,
    content: String,
    provider: &str,
    turn_index: u32,
    tool_name: Option<String>,
) -> Block {
    let mut block = make_block(role, content, provider, turn_index);
    block.metadata.tool_name = tool_name;
    block
}

/// Default zone assignment based on role.
fn default_zone_for_role(role: Role) -> Zone {
    use crate::engine::types::BuiltInZone;
    match role {
        Role::System => Zone::BuiltIn(BuiltInZone::Primacy),
        _ => Zone::BuiltIn(BuiltInZone::Recency),
    }
}

// ============================================================================
// Anthropic parser
// ============================================================================

/// Parse an Anthropic Messages API request body into blocks.
pub fn parse_anthropic_request(body: &[u8]) -> Result<ParsedRequest, String> {
    let raw: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("Invalid Anthropic request JSON: {e}"))?;
    let stream = raw.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let req: AnthropicRequest = serde_json::from_value(raw)
        .map_err(|e| format!("Invalid Anthropic request structure: {e}"))?;

    let provider_str = "anthropic";
    let mut blocks = Vec::new();
    let mut system_prompt = None;

    // Handle system prompt (can be string or array of content blocks)
    if let Some(system) = &req.system {
        let system_content = match system {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(arr) => {
                // Array of {type: "text", text: "..."} blocks
                arr.iter()
                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => String::new(),
        };
        if !system_content.is_empty() {
            system_prompt = Some(system_content.clone());
            blocks.push(make_block(Role::System, system_content, provider_str, 0));
        }
    }

    // Parse messages
    for (i, msg) in req.messages.iter().enumerate() {
        let turn_index = (i + 1) as u32; // 0 is system
        let parsed = parse_anthropic_message(msg, provider_str, turn_index);
        blocks.extend(parsed);
    }

    Ok(ParsedRequest {
        provider: Provider::Anthropic,
        model: req.model,
        blocks,
        system_prompt,
        stream,
    })
}

/// Parse a single Anthropic message into one or more blocks.
fn parse_anthropic_message(msg: &AnthropicMessage, provider: &str, turn_index: u32) -> Vec<Block> {
    let mut blocks = Vec::new();

    match &msg.content {
        // Simple string content
        serde_json::Value::String(text) => {
            let role = match msg.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => Role::User,
            };
            blocks.push(make_block(role, text.clone(), provider, turn_index));
        }
        // Array of content blocks
        serde_json::Value::Array(content_blocks) => {
            for cb_value in content_blocks {
                if let Ok(cb) = serde_json::from_value::<AnthropicContentBlock>(cb_value.clone()) {
                    match cb.content_type.as_str() {
                        "text" => {
                            if let Some(text) = cb.text {
                                let role = match msg.role.as_str() {
                                    "user" => Role::User,
                                    "assistant" => Role::Assistant,
                                    "system" => Role::System,
                                    _ => Role::User,
                                };
                                blocks.push(make_block(role, text, provider, turn_index));
                            }
                        }
                        "tool_use" => {
                            let input_str = cb
                                .input
                                .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
                                .unwrap_or_default();
                            let content = format!(
                                "Tool: {}\nID: {}\nInput:\n{}",
                                cb.name.as_deref().unwrap_or("unknown"),
                                cb.id.as_deref().unwrap_or(""),
                                input_str
                            );
                            blocks.push(make_tool_block(
                                Role::ToolUse,
                                content,
                                provider,
                                turn_index,
                                cb.name,
                            ));
                        }
                        "tool_result" => {
                            let result_content = match &cb.content {
                                Some(serde_json::Value::String(s)) => s.clone(),
                                Some(serde_json::Value::Array(arr)) => arr
                                    .iter()
                                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                Some(v) => serde_json::to_string_pretty(v).unwrap_or_default(),
                                None => String::new(),
                            };
                            let content = format!(
                                "Tool Result ({})\n{}",
                                cb.tool_use_id.as_deref().unwrap_or(""),
                                result_content
                            );
                            blocks.push(make_tool_block(
                                Role::ToolResult,
                                content,
                                provider,
                                turn_index,
                                None,
                            ));
                        }
                        "thinking" => {
                            let thinking_text = cb.thinking.or(cb.text).unwrap_or_default();
                            if !thinking_text.is_empty() {
                                blocks.push(make_block(
                                    Role::Thinking,
                                    thinking_text,
                                    provider,
                                    turn_index,
                                ));
                            }
                        }
                        "image" => {
                            blocks.push(make_block(
                                Role::User,
                                "[Image content]".to_string(),
                                provider,
                                turn_index,
                            ));
                        }
                        _ => {
                            // Unknown content type — preserve as JSON
                            let content =
                                serde_json::to_string_pretty(cb_value).unwrap_or_default();
                            let role = match msg.role.as_str() {
                                "assistant" => Role::Assistant,
                                _ => Role::User,
                            };
                            blocks.push(make_block(role, content, provider, turn_index));
                        }
                    }
                }
            }
        }
        // Null or other — skip
        _ => {}
    }

    blocks
}

/// Parse an Anthropic Messages API response body.
pub fn parse_anthropic_response(body: &[u8]) -> Result<ParsedResponse, String> {
    let resp: AnthropicResponse = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid Anthropic response JSON: {e}"))?;

    let provider_str = "anthropic";
    let mut blocks = Vec::new();

    for content in &resp.content {
        match content.content_type.as_str() {
            "text" => {
                if let Some(text) = &content.text {
                    blocks.push(make_block(Role::Assistant, text.clone(), provider_str, 0));
                }
            }
            "tool_use" => {
                let input_str = content
                    .input
                    .as_ref()
                    .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                    .unwrap_or_default();
                let text = format!(
                    "Tool: {}\nID: {}\nInput:\n{}",
                    content.name.as_deref().unwrap_or("unknown"),
                    content.id.as_deref().unwrap_or(""),
                    input_str
                );
                blocks.push(make_tool_block(
                    Role::ToolUse,
                    text,
                    provider_str,
                    0,
                    content.name.clone(),
                ));
            }
            "thinking" => {
                let thinking_text = content
                    .thinking
                    .as_deref()
                    .or(content.text.as_deref())
                    .unwrap_or_default();
                if !thinking_text.is_empty() {
                    blocks.push(make_block(
                        Role::Thinking,
                        thinking_text.to_string(),
                        provider_str,
                        0,
                    ));
                }
            }
            _ => {}
        }
    }

    let usage = resp.usage.map(|u| TokenUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
    });

    Ok(ParsedResponse {
        provider: Provider::Anthropic,
        blocks,
        usage,
        model: resp.model,
    })
}

// ============================================================================
// OpenAI Chat Completions parser
// ============================================================================

/// Parse an OpenAI Chat Completions request body into blocks.
pub fn parse_openai_chat_request(body: &[u8]) -> Result<ParsedRequest, String> {
    let raw: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid OpenAI chat request JSON: {e}"))?;
    let stream = raw.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let req: OpenAIChatRequest = serde_json::from_value(raw)
        .map_err(|e| format!("Invalid OpenAI chat request structure: {e}"))?;

    let provider_str = "openai";
    let mut blocks = Vec::new();

    for (i, msg) in req.messages.iter().enumerate() {
        let turn_index = i as u32;
        let parsed = parse_openai_chat_message(msg, provider_str, turn_index);
        blocks.extend(parsed);
    }

    Ok(ParsedRequest {
        provider: Provider::OpenAI,
        model: req.model,
        blocks,
        system_prompt: None, // OpenAI system prompt is in messages array
        stream,
    })
}

/// Parse a single OpenAI chat message into one or more blocks.
fn parse_openai_chat_message(
    msg: &OpenAIChatMessage,
    provider: &str,
    turn_index: u32,
) -> Vec<Block> {
    let mut blocks = Vec::new();

    // Handle tool_calls (assistant making tool calls)
    if let Some(tool_calls) = &msg.tool_calls {
        for tc in tool_calls {
            let content = format!(
                "Tool: {}\nID: {}\nArguments:\n{}",
                tc.function.name, tc.id, tc.function.arguments
            );
            blocks.push(make_tool_block(
                Role::ToolUse,
                content,
                provider,
                turn_index,
                Some(tc.function.name.clone()),
            ));
        }
        // If there's also text content alongside tool_calls, parse it
        if let Some(content_val) = &msg.content {
            if let Some(text) = extract_text_content(content_val) {
                if !text.is_empty() {
                    blocks.push(make_block(Role::Assistant, text, provider, turn_index));
                }
            }
        }
        return blocks;
    }

    // Handle tool response
    if msg.role == "tool" {
        let content_text = msg
            .content
            .as_ref()
            .and_then(extract_text_content)
            .unwrap_or_default();
        let content = format!(
            "Tool Result ({})\n{}",
            msg.tool_call_id.as_deref().unwrap_or(""),
            content_text
        );
        blocks.push(make_tool_block(
            Role::ToolResult,
            content,
            provider,
            turn_index,
            msg.name.clone(),
        ));
        return blocks;
    }

    // Regular message
    let role = match msg.role.as_str() {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => Role::User,
    };

    if let Some(content_val) = &msg.content {
        if let Some(text) = extract_text_content(content_val) {
            if !text.is_empty() {
                blocks.push(make_block(role, text, provider, turn_index));
            }
        }
    }

    blocks
}

/// Extract text content from OpenAI content value (string or array of parts).
fn extract_text_content(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => {
            let texts: Vec<&str> = parts
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                        p.get("text").and_then(|t| t.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        serde_json::Value::Null => None,
        _ => Some(value.to_string()),
    }
}

/// Parse an OpenAI Chat Completions response body.
pub fn parse_openai_chat_response(body: &[u8]) -> Result<ParsedResponse, String> {
    let resp: OpenAIChatResponse = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid OpenAI chat response JSON: {e}"))?;

    let provider_str = "openai";
    let mut blocks = Vec::new();

    for choice in &resp.choices {
        let parsed = parse_openai_chat_message(&choice.message, provider_str, 0);
        blocks.extend(parsed);
    }

    let usage = resp.usage.map(|u| TokenUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
    });

    Ok(ParsedResponse {
        provider: Provider::OpenAI,
        blocks,
        usage,
        model: resp.model,
    })
}

// ============================================================================
// OpenAI Responses API parser
// ============================================================================

/// Parse an OpenAI Responses API request body into blocks.
pub fn parse_openai_responses_request(body: &[u8]) -> Result<ParsedRequest, String> {
    let raw: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid OpenAI responses request JSON: {e}"))?;
    let stream = raw.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let req: OpenAIResponsesRequest = serde_json::from_value(raw)
        .map_err(|e| format!("Invalid OpenAI responses request structure: {e}"))?;

    let provider_str = "openai";
    let mut blocks = Vec::new();

    // Handle instructions as system prompt
    if let Some(instructions) = &req.instructions {
        if !instructions.is_empty() {
            blocks.push(make_block(
                Role::System,
                instructions.clone(),
                provider_str,
                0,
            ));
        }
    }

    // Parse input items
    match &req.input {
        serde_json::Value::String(text) => {
            blocks.push(make_block(Role::User, text.clone(), provider_str, 1));
        }
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let turn_index = (i + 1) as u32;
                if let Some(item_type) = item.get("type").and_then(|t| t.as_str()) {
                    match item_type {
                        "message" => {
                            let role_str =
                                item.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                            let role = match role_str {
                                "system" => Role::System,
                                "user" => Role::User,
                                "assistant" => Role::Assistant,
                                _ => Role::User,
                            };
                            // Content can be string or array
                            if let Some(content) = item.get("content") {
                                if let Some(text) = extract_text_content(content) {
                                    blocks.push(make_block(role, text, provider_str, turn_index));
                                }
                            }
                        }
                        "function_call_output" => {
                            let output = item
                                .get("output")
                                .and_then(|o| o.as_str())
                                .unwrap_or("")
                                .to_string();
                            let call_id = item
                                .get("call_id")
                                .and_then(|c| c.as_str())
                                .unwrap_or("")
                                .to_string();
                            let content = format!("Tool Result ({call_id})\n{output}");
                            blocks.push(make_tool_block(
                                Role::ToolResult,
                                content,
                                provider_str,
                                turn_index,
                                None,
                            ));
                        }
                        _ => {
                            // Unknown item type — preserve as JSON
                            let content = serde_json::to_string_pretty(item).unwrap_or_default();
                            blocks.push(make_block(Role::User, content, provider_str, turn_index));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(ParsedRequest {
        provider: Provider::OpenAI,
        model: req.model,
        blocks,
        system_prompt: req.instructions,
        stream,
    })
}

/// Parse an OpenAI Responses API response body.
pub fn parse_openai_responses_response(body: &[u8]) -> Result<ParsedResponse, String> {
    let resp: OpenAIResponsesResponse = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid OpenAI responses response JSON: {e}"))?;

    let provider_str = "openai";
    let mut blocks = Vec::new();

    for output in &resp.output {
        match output.output_type.as_str() {
            "message" => {
                if let Some(content_items) = &output.content {
                    for item in content_items {
                        if item.content_type == "output_text" || item.content_type == "text" {
                            if let Some(text) = &item.text {
                                blocks.push(make_block(
                                    Role::Assistant,
                                    text.clone(),
                                    provider_str,
                                    0,
                                ));
                            }
                        }
                    }
                }
            }
            "function_call" => {
                let content = format!(
                    "Tool: {}\nID: {}\nArguments:\n{}",
                    output.name.as_deref().unwrap_or("unknown"),
                    output.call_id.as_deref().unwrap_or(""),
                    output.arguments.as_deref().unwrap_or("{}")
                );
                blocks.push(make_tool_block(
                    Role::ToolUse,
                    content,
                    provider_str,
                    0,
                    output.name.clone(),
                ));
            }
            _ => {}
        }
    }

    let usage = resp.usage.map(|u| TokenUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
    });

    Ok(ParsedResponse {
        provider: Provider::OpenAI,
        blocks,
        usage,
        model: resp.model,
    })
}

// ============================================================================
// Unified parser dispatch
// ============================================================================

/// Parse a request body, auto-detecting format from the path.
pub fn parse_request(path: &str, body: &[u8]) -> Result<ParsedRequest, String> {
    if body.is_empty() {
        return Err("Empty request body".to_string());
    }

    if is_messages_path(path) {
        parse_anthropic_request(body)
    } else if is_chat_completions_path(path) {
        parse_openai_chat_request(body)
    } else if is_responses_path(path) {
        parse_openai_responses_request(body)
    } else {
        // Try Anthropic first (primary use case), fall back to OpenAI
        parse_anthropic_request(body).or_else(|_| parse_openai_chat_request(body))
    }
}

/// Parse a response body, using the known provider.
pub fn parse_response(
    provider: Provider,
    path: &str,
    body: &[u8],
) -> Result<ParsedResponse, String> {
    if body.is_empty() {
        return Ok(ParsedResponse {
            provider,
            blocks: Vec::new(),
            usage: None,
            model: None,
        });
    }

    match provider {
        Provider::Anthropic => parse_anthropic_response(body),
        Provider::OpenAI => {
            if is_responses_path(path) {
                parse_openai_responses_response(body)
            } else {
                parse_openai_chat_response(body)
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Provider detection ---

    #[test]
    fn test_provider_from_path_anthropic() {
        assert_eq!(
            Provider::from_path("/v1/messages"),
            Some(Provider::Anthropic)
        );
    }

    #[test]
    fn test_provider_from_path_openai_chat() {
        assert_eq!(
            Provider::from_path("/v1/chat/completions"),
            Some(Provider::OpenAI)
        );
    }

    #[test]
    fn test_provider_from_path_openai_responses() {
        assert_eq!(Provider::from_path("/v1/responses"), Some(Provider::OpenAI));
    }

    #[test]
    fn test_provider_from_path_unknown() {
        assert_eq!(Provider::from_path("/v1/unknown"), None);
    }

    // --- Bare path detection (no /v1/ prefix) ---

    #[test]
    fn test_provider_from_bare_responses_path() {
        assert_eq!(Provider::from_path("/responses"), Some(Provider::OpenAI));
    }

    #[test]
    fn test_provider_from_bare_chat_completions_path() {
        assert_eq!(
            Provider::from_path("/chat/completions"),
            Some(Provider::OpenAI)
        );
    }

    #[test]
    fn test_provider_from_bare_messages_path() {
        assert_eq!(Provider::from_path("/messages"), Some(Provider::Anthropic));
    }

    #[test]
    fn test_provider_from_responses_subpath() {
        assert_eq!(
            Provider::from_path("/responses/resp_123/cancel"),
            Some(Provider::OpenAI)
        );
    }

    #[test]
    fn test_provider_from_messages_subpath() {
        assert_eq!(
            Provider::from_path("/v1/messages/count_tokens"),
            Some(Provider::Anthropic)
        );
    }

    #[test]
    fn test_parse_request_bare_responses_path() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "input": "test"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_request("/responses", &body_bytes).unwrap();
        assert_eq!(result.provider, Provider::OpenAI);
    }

    #[test]
    fn test_parse_request_bare_chat_completions_path() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_request("/chat/completions", &body_bytes).unwrap();
        assert_eq!(result.provider, Provider::OpenAI);
    }

    #[test]
    fn test_parse_request_bare_messages_path() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_request("/messages", &body_bytes).unwrap();
        assert_eq!(result.provider, Provider::Anthropic);
    }

    // --- Token estimation ---

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short_text() {
        // "hello" = 5 chars, ceil(5/4) = 2
        assert_eq!(estimate_tokens("hello"), 2);
    }

    // --- Anthropic request parsing ---

    #[test]
    fn test_parse_anthropic_request_simple() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Hello, Claude!"}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.provider, Provider::Anthropic);
        assert_eq!(result.model, "claude-sonnet-4-20250514");
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].role, Role::User);
        assert_eq!(result.blocks[0].content, "Hello, Claude!");
        assert!(result.blocks[0].tokens > 0);
    }

    #[test]
    fn test_parse_anthropic_request_with_system() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "system": "You are a helpful assistant.",
            "messages": [
                {"role": "user", "content": "Hi"}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].role, Role::System);
        assert_eq!(result.blocks[0].content, "You are a helpful assistant.");
        assert_eq!(result.blocks[1].role, Role::User);
        assert_eq!(
            result.system_prompt,
            Some("You are a helpful assistant.".to_string())
        );
    }

    #[test]
    fn test_parse_anthropic_request_with_system_array() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "You are"},
                {"type": "text", "text": "helpful"}
            ],
            "messages": []
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].content, "You are\nhelpful");
    }

    #[test]
    fn test_parse_anthropic_request_tool_use() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_123", "name": "get_weather", "input": {"location": "SF"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_123", "content": "72°F and sunny"}
                ]}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 3);
        assert_eq!(result.blocks[0].role, Role::User);
        assert_eq!(result.blocks[1].role, Role::ToolUse);
        assert!(result.blocks[1].content.contains("get_weather"));
        assert_eq!(
            result.blocks[1].metadata.tool_name,
            Some("get_weather".to_string())
        );
        assert_eq!(result.blocks[2].role, Role::ToolResult);
        assert!(result.blocks[2].content.contains("72°F"));
    }

    #[test]
    fn test_parse_anthropic_request_multi_turn() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {"role": "user", "content": "How are you?"},
                {"role": "assistant", "content": "I'm doing well!"}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 4);
        assert_eq!(result.blocks[0].role, Role::User);
        assert_eq!(result.blocks[1].role, Role::Assistant);
        assert_eq!(result.blocks[2].role, Role::User);
        assert_eq!(result.blocks[3].role, Role::Assistant);
        // Turn indices should increment
        assert_eq!(result.blocks[0].metadata.turn_index, 1);
        assert_eq!(result.blocks[3].metadata.turn_index, 4);
    }

    // --- Anthropic response parsing ---

    #[test]
    fn test_parse_anthropic_response_text() {
        let body = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-20250514",
            "content": [
                {"type": "text", "text": "Hello! How can I help?"}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 8}
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_response(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].role, Role::Assistant);
        assert_eq!(result.blocks[0].content, "Hello! How can I help?");
        assert_eq!(result.usage.as_ref().unwrap().input_tokens, 10);
        assert_eq!(result.usage.as_ref().unwrap().output_tokens, 8);
        assert_eq!(result.model, Some("claude-sonnet-4-20250514".to_string()));
    }

    #[test]
    fn test_parse_anthropic_response_tool_use() {
        let body = serde_json::json!({
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "tool_use", "id": "toolu_789", "name": "read_file", "input": {"path": "/tmp/test.txt"}}
            ],
            "usage": {"input_tokens": 20, "output_tokens": 15}
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_response(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].role, Role::ToolUse);
        assert!(result.blocks[0].content.contains("read_file"));
        assert_eq!(
            result.blocks[0].metadata.tool_name,
            Some("read_file".to_string())
        );
    }

    // --- OpenAI Chat Completions parsing ---

    #[test]
    fn test_parse_openai_chat_request_simple() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello!"}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_openai_chat_request(&body_bytes).unwrap();
        assert_eq!(result.provider, Provider::OpenAI);
        assert_eq!(result.model, "gpt-4");
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].role, Role::System);
        assert_eq!(result.blocks[1].role, Role::User);
    }

    #[test]
    fn test_parse_openai_chat_request_tool_calls() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"location\":\"SF\"}"}
                    }]
                },
                {"role": "tool", "tool_call_id": "call_abc", "content": "72°F"}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_openai_chat_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 3);
        assert_eq!(result.blocks[0].role, Role::User);
        assert_eq!(result.blocks[1].role, Role::ToolUse);
        assert!(result.blocks[1].content.contains("get_weather"));
        assert_eq!(result.blocks[2].role, Role::ToolResult);
        assert!(result.blocks[2].content.contains("72°F"));
    }

    #[test]
    fn test_parse_openai_chat_response() {
        let body = serde_json::json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_openai_chat_response(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].role, Role::Assistant);
        assert_eq!(result.blocks[0].content, "Hello!");
        assert_eq!(result.usage.as_ref().unwrap().input_tokens, 10);
        assert_eq!(result.usage.as_ref().unwrap().output_tokens, 5);
    }

    // --- OpenAI Responses API parsing ---

    #[test]
    fn test_parse_openai_responses_request_string_input() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "input": "Hello!",
            "instructions": "You are helpful."
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_openai_responses_request(&body_bytes).unwrap();
        assert_eq!(result.provider, Provider::OpenAI);
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].role, Role::System);
        assert_eq!(result.blocks[0].content, "You are helpful.");
        assert_eq!(result.blocks[1].role, Role::User);
        assert_eq!(result.blocks[1].content, "Hello!");
    }

    #[test]
    fn test_parse_openai_responses_request_message_items() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "input": [
                {"type": "message", "role": "user", "content": "What's 2+2?"},
                {"type": "message", "role": "assistant", "content": "4"}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_openai_responses_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].role, Role::User);
        assert_eq!(result.blocks[1].role, Role::Assistant);
    }

    #[test]
    fn test_parse_openai_responses_response() {
        let body = serde_json::json!({
            "id": "resp_123",
            "model": "gpt-4",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "The answer is 4."}]
            }],
            "usage": {"input_tokens": 15, "output_tokens": 8}
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_openai_responses_response(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].content, "The answer is 4.");
        assert_eq!(result.usage.as_ref().unwrap().input_tokens, 15);
        assert_eq!(result.usage.as_ref().unwrap().output_tokens, 8);
    }

    #[test]
    fn test_parse_openai_responses_response_function_call() {
        let body = serde_json::json!({
            "id": "resp_456",
            "output": [{
                "type": "function_call",
                "name": "get_weather",
                "call_id": "call_xyz",
                "arguments": "{\"location\": \"NYC\"}"
            }],
            "usage": {"input_tokens": 10, "output_tokens": 12}
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_openai_responses_response(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].role, Role::ToolUse);
        assert!(result.blocks[0].content.contains("get_weather"));
    }

    // --- Unified dispatch ---

    #[test]
    fn test_parse_request_anthropic_path() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_request("/v1/messages", &body_bytes).unwrap();
        assert_eq!(result.provider, Provider::Anthropic);
    }

    #[test]
    fn test_parse_request_openai_chat_path() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_request("/v1/chat/completions", &body_bytes).unwrap();
        assert_eq!(result.provider, Provider::OpenAI);
    }

    #[test]
    fn test_parse_request_openai_responses_path() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "input": "test"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_request("/v1/responses", &body_bytes).unwrap();
        assert_eq!(result.provider, Provider::OpenAI);
    }

    #[test]
    fn test_parse_request_empty_body() {
        let result = parse_request("/v1/messages", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_response_empty_body() {
        let result = parse_response(Provider::Anthropic, "/v1/messages", &[]).unwrap();
        assert!(result.blocks.is_empty());
        assert!(result.usage.is_none());
    }

    // --- Zone assignment ---

    #[test]
    fn test_system_blocks_go_to_primacy() {
        use crate::engine::types::BuiltInZone;
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "system": "Be helpful.",
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy));
        assert_eq!(result.blocks[1].zone, Zone::BuiltIn(BuiltInZone::Recency));
    }

    // --- Edge cases ---

    #[test]
    fn test_parse_anthropic_request_image_content() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What's in this?"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}}
                ]
            }]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].content, "What's in this?");
        assert_eq!(result.blocks[1].content, "[Image content]");
    }

    #[test]
    fn test_parse_anthropic_request_invalid_json() {
        let result = parse_anthropic_request(b"not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_block_metadata_populated() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        let block = &result.blocks[0];
        assert_eq!(block.metadata.provider, "anthropic");
        assert_eq!(block.metadata.turn_index, 1);
        assert!(!block.id.is_empty());
        assert!(!block.timestamp.is_empty());
    }

    // --- Thinking blocks ---

    #[test]
    fn test_parse_anthropic_request_thinking_block() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Solve this math problem"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "Let me work through this step by step..."},
                    {"type": "text", "text": "The answer is 42."}
                ]}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 3);
        assert_eq!(result.blocks[0].role, Role::User);
        assert_eq!(result.blocks[1].role, Role::Thinking);
        assert_eq!(
            result.blocks[1].content,
            "Let me work through this step by step..."
        );
        assert_eq!(result.blocks[2].role, Role::Assistant);
        assert_eq!(result.blocks[2].content, "The answer is 42.");
    }

    #[test]
    fn test_parse_anthropic_response_thinking_block() {
        let body = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-20250514",
            "content": [
                {"type": "thinking", "thinking": "I need to consider the implications..."},
                {"type": "text", "text": "Here is my analysis."}
            ],
            "usage": {"input_tokens": 20, "output_tokens": 30}
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_response(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].role, Role::Thinking);
        assert_eq!(
            result.blocks[0].content,
            "I need to consider the implications..."
        );
        assert_eq!(result.blocks[1].role, Role::Assistant);
        assert_eq!(result.blocks[1].content, "Here is my analysis.");
    }

    #[test]
    fn test_parse_anthropic_thinking_with_signature_ignored() {
        // Thinking blocks may have a signature field — we should parse the thinking
        // content and ignore the signature (it's for verification, not display)
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [
                {"role": "assistant", "content": [
                    {
                        "type": "thinking",
                        "thinking": "Deep analysis here...",
                        "signature": "sig_abc123"
                    }
                ]}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let result = parse_anthropic_request(&body_bytes).unwrap();
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].role, Role::Thinking);
        assert_eq!(result.blocks[0].content, "Deep analysis here...");
    }

    // --- Stream field extraction ---

    #[test]
    fn test_parse_anthropic_stream_true() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let result = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(result.stream);
    }

    #[test]
    fn test_parse_anthropic_stream_false() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "stream": false,
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let result = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(!result.stream);
    }

    #[test]
    fn test_parse_anthropic_stream_absent() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let result = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(!result.stream);
    }

    #[test]
    fn test_parse_openai_chat_stream_true() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "stream": true,
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let result = parse_openai_chat_request(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(result.stream);
    }

    #[test]
    fn test_parse_openai_chat_stream_absent() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let result = parse_openai_chat_request(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(!result.stream);
    }

    #[test]
    fn test_parse_openai_responses_stream_true() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "stream": true,
            "input": "Hello"
        });
        let result = parse_openai_responses_request(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(result.stream);
    }

    #[test]
    fn test_parse_openai_responses_stream_absent() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "input": "Hello"
        });
        let result = parse_openai_responses_request(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(!result.stream);
    }
}
