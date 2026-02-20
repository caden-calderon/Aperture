use serde::Deserialize;

use super::*;

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
// Anthropic parser
// ============================================================================

/// Parse an Anthropic Messages API request body into blocks.
pub fn parse_anthropic_request(body: &[u8]) -> Result<ParsedRequest, String> {
    let raw: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("Invalid Anthropic request JSON: {e}"))?;
    let stream = raw.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let overhead_tokens = estimate_request_overhead(&raw);

    let req: AnthropicRequest = serde_json::from_value(raw.clone())
        .map_err(|e| format!("Invalid Anthropic request structure: {e}"))?;

    let provider_str = "anthropic";
    let mut blocks = Vec::new();
    let mut system_prompt = None;
    let mut tracker = OccurrenceTracker::new();

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
            // Normalize system content for stable fingerprinting by filtering
            // dynamic headers that change per request (e.g. billing headers).
            let fingerprint_content: String = system_content
                .lines()
                .filter(|line| {
                    !line
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("x-anthropic-billing-header:")
                })
                .collect::<Vec<_>>()
                .join("\n");
            let fp = content_fingerprint(&fingerprint_content);
            let occ = tracker.next(Role::System, &fp);
            let block_key = format!("anthropic:system:{occ}");
            blocks.push(make_block(
                Role::System,
                system_content,
                provider_str,
                0,
                &fp,
                &block_key,
            ));
        }
    }

    // Parse messages
    for (i, msg) in req.messages.iter().enumerate() {
        let turn_index = (i + 1) as u32; // 0 is system
        let parsed = parse_anthropic_message(msg, provider_str, turn_index, &mut tracker);
        blocks.extend(parsed);
    }

    let thread_identity = derive_thread_identity(&raw, &blocks);

    Ok(ParsedRequest {
        provider: Provider::Anthropic,
        model: req.model,
        blocks,
        thread_identity,
        system_prompt,
        stream,
        overhead_tokens,
    })
}

/// Parse a single Anthropic message into one or more blocks.
fn parse_anthropic_message(
    msg: &AnthropicMessage,
    provider: &str,
    turn_index: u32,
    tracker: &mut OccurrenceTracker,
) -> Vec<Block> {
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
            let fp = content_fingerprint(text);
            let occ = tracker.next(role, &fp);
            let block_key = format!("anthropic:text:0:{occ}");
            blocks.push(make_block(
                role,
                text.clone(),
                provider,
                turn_index,
                &fp,
                &block_key,
            ));
        }
        // Array of content blocks
        serde_json::Value::Array(content_blocks) => {
            for (content_index, cb_value) in content_blocks.iter().enumerate() {
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
                                let fp = content_fingerprint(&text);
                                let occ = tracker.next(role, &fp);
                                let block_key = format!("anthropic:text:{content_index}:{occ}");
                                blocks.push(make_block(
                                    role, text, provider, turn_index, &fp, &block_key,
                                ));
                            }
                        }
                        "tool_use" => {
                            let tool_use_id = cb.id.as_deref().unwrap_or("");
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
                            let fp = content_fingerprint(&content);
                            let occ = tracker.next(Role::ToolUse, &fp);
                            let block_key = format!("anthropic:tool_use:{tool_use_id}:{occ}");
                            blocks.push(make_tool_block(
                                Role::ToolUse,
                                content,
                                provider,
                                turn_index,
                                cb.name,
                                &fp,
                                &block_key,
                            ));
                        }
                        "tool_result" => {
                            let tool_use_id = cb.tool_use_id.as_deref().unwrap_or("");
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
                            let fp = content_fingerprint(&content);
                            let occ = tracker.next(Role::ToolResult, &fp);
                            let block_key = format!("anthropic:tool_result:{tool_use_id}:{occ}");
                            blocks.push(make_tool_block(
                                Role::ToolResult,
                                content,
                                provider,
                                turn_index,
                                None,
                                &fp,
                                &block_key,
                            ));
                        }
                        "thinking" => {
                            let thinking_text = cb.thinking.or(cb.text).unwrap_or_default();
                            if !thinking_text.is_empty() {
                                let fp = content_fingerprint(&thinking_text);
                                let occ = tracker.next(Role::Thinking, &fp);
                                let block_key = format!("anthropic:thinking:{content_index}:{occ}");
                                blocks.push(make_block(
                                    Role::Thinking,
                                    thinking_text,
                                    provider,
                                    turn_index,
                                    &fp,
                                    &block_key,
                                ));
                            }
                        }
                        "image" => {
                            let img_content = "[Image content]".to_string();
                            let fp = content_fingerprint(&img_content);
                            let occ = tracker.next(Role::User, &fp);
                            let block_key = format!("anthropic:image:{content_index}:{occ}");
                            blocks.push(make_block(
                                Role::User,
                                img_content,
                                provider,
                                turn_index,
                                &fp,
                                &block_key,
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
                            let fp = content_fingerprint(&content);
                            let occ = tracker.next(role, &fp);
                            let block_key = format!("anthropic:unknown:{content_index}:{occ}");
                            blocks.push(make_block(
                                role, content, provider, turn_index, &fp, &block_key,
                            ));
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
    let mut tracker = OccurrenceTracker::new();

    for content in resp.content.iter() {
        match content.content_type.as_str() {
            "text" => {
                if let Some(text) = &content.text {
                    let fp = content_fingerprint(text);
                    let occ = tracker.next(Role::Assistant, &fp);
                    let block_key = format!("anthropic:response:text:{occ}");
                    blocks.push(make_block(
                        Role::Assistant,
                        text.clone(),
                        provider_str,
                        0,
                        &fp,
                        &block_key,
                    ));
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
                let tool_use_id = content.id.as_deref().unwrap_or("");
                let fp = content_fingerprint(&text);
                let occ = tracker.next(Role::ToolUse, &fp);
                let block_key = format!("anthropic:response:tool_use:{tool_use_id}:{occ}");
                blocks.push(make_tool_block(
                    Role::ToolUse,
                    text,
                    provider_str,
                    0,
                    content.name.clone(),
                    &fp,
                    &block_key,
                ));
            }
            "thinking" => {
                let thinking_text = content
                    .thinking
                    .as_deref()
                    .or(content.text.as_deref())
                    .unwrap_or_default();
                if !thinking_text.is_empty() {
                    let fp = content_fingerprint(thinking_text);
                    let occ = tracker.next(Role::Thinking, &fp);
                    let block_key = format!("anthropic:response:thinking:{occ}");
                    blocks.push(make_block(
                        Role::Thinking,
                        thinking_text.to_string(),
                        provider_str,
                        0,
                        &fp,
                        &block_key,
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
