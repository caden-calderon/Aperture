use serde::Deserialize;

use super::*;

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
// OpenAI Chat Completions parser
// ============================================================================

/// Parse an OpenAI Chat Completions request body into blocks.
pub fn parse_openai_chat_request(body: &[u8]) -> Result<ParsedRequest, String> {
    let raw: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid OpenAI chat request JSON: {e}"))?;
    let stream = raw.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let overhead_tokens = estimate_request_overhead(&raw);

    let req: OpenAIChatRequest = serde_json::from_value(raw.clone())
        .map_err(|e| format!("Invalid OpenAI chat request structure: {e}"))?;

    let provider_str = "openai";
    let mut blocks = Vec::new();
    let mut tracker = OccurrenceTracker::new();

    for (i, msg) in req.messages.iter().enumerate() {
        let turn_index = i as u32;
        let parsed = parse_openai_chat_message(msg, provider_str, turn_index, &mut tracker);
        blocks.extend(parsed);
    }

    let thread_identity = derive_thread_identity(&raw, &blocks);

    Ok(ParsedRequest {
        provider: Provider::OpenAI,
        model: req.model,
        thread_identity,
        blocks,
        system_prompt: None, // OpenAI system prompt is in messages array
        stream,
        overhead_tokens,
    })
}

/// Parse a single OpenAI chat message into one or more blocks.
fn parse_openai_chat_message(
    msg: &OpenAIChatMessage,
    provider: &str,
    turn_index: u32,
    tracker: &mut OccurrenceTracker,
) -> Vec<Block> {
    let mut blocks = Vec::new();

    // Handle tool_calls (assistant making tool calls)
    if let Some(tool_calls) = &msg.tool_calls {
        for tc in tool_calls.iter() {
            let content = format!(
                "Tool: {}\nID: {}\nArguments:\n{}",
                tc.function.name, tc.id, tc.function.arguments
            );
            let fp = content_fingerprint(&content);
            let occ = tracker.next(Role::ToolUse, &fp);
            let block_key = format!("openai:chat:tool_call:{}:{occ}", tc.id);
            blocks.push(make_tool_block(
                Role::ToolUse,
                content,
                provider,
                turn_index,
                Some(tc.function.name.clone()),
                &fp,
                &block_key,
            ));
        }
        // If there's also text content alongside tool_calls, parse it
        if let Some(content_val) = &msg.content {
            if let Some(text) = extract_text_content(content_val) {
                if !text.is_empty() {
                    let fp = content_fingerprint(&text);
                    let occ = tracker.next(Role::Assistant, &fp);
                    let block_key = format!("openai:chat:assistant_text:{occ}");
                    blocks.push(make_block(
                        Role::Assistant,
                        text,
                        provider,
                        turn_index,
                        &fp,
                        &block_key,
                    ));
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
        let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("");
        let content = format!("Tool Result ({tool_call_id})\n{content_text}");
        let fp = content_fingerprint(&content);
        let occ = tracker.next(Role::ToolResult, &fp);
        let block_key = format!("openai:chat:tool_result:{tool_call_id}:{occ}");
        blocks.push(make_tool_block(
            Role::ToolResult,
            content,
            provider,
            turn_index,
            msg.name.clone(),
            &fp,
            &block_key,
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
                let fp = content_fingerprint(&text);
                let occ = tracker.next(role, &fp);
                let block_key = format!("openai:chat:text:{occ}");
                blocks.push(make_block(
                    role, text, provider, turn_index, &fp, &block_key,
                ));
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
    let mut tracker = OccurrenceTracker::new();

    for choice in resp.choices.iter() {
        let parsed = parse_openai_chat_message(&choice.message, provider_str, 0, &mut tracker);
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
    let overhead_tokens = estimate_request_overhead(&raw);

    let req: OpenAIResponsesRequest = serde_json::from_value(raw.clone())
        .map_err(|e| format!("Invalid OpenAI responses request structure: {e}"))?;

    let provider_str = "openai";
    let mut blocks = Vec::new();
    let mut tracker = OccurrenceTracker::new();

    // Handle instructions as system prompt
    if let Some(instructions) = &req.instructions {
        if !instructions.is_empty() {
            let fp = content_fingerprint(instructions);
            let occ = tracker.next(Role::System, &fp);
            let block_key = format!("openai:responses:instructions:{occ}");
            blocks.push(make_block(
                Role::System,
                instructions.clone(),
                provider_str,
                0,
                &fp,
                &block_key,
            ));
        }
    }

    // Parse input items
    match &req.input {
        serde_json::Value::String(text) => {
            let fp = content_fingerprint(text);
            let occ = tracker.next(Role::User, &fp);
            let block_key = format!("openai:responses:input:{occ}");
            blocks.push(make_block(
                Role::User,
                text.clone(),
                provider_str,
                1,
                &fp,
                &block_key,
            ));
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
                                    let fp = content_fingerprint(&text);
                                    let occ = tracker.next(role, &fp);
                                    let block_key = format!("openai:responses:message:{occ}");
                                    blocks.push(make_block(
                                        role,
                                        text,
                                        provider_str,
                                        turn_index,
                                        &fp,
                                        &block_key,
                                    ));
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
                            let fp = content_fingerprint(&content);
                            let occ = tracker.next(Role::ToolResult, &fp);
                            let block_key =
                                format!("openai:responses:function_call_output:{call_id}:{occ}");
                            blocks.push(make_tool_block(
                                Role::ToolResult,
                                content,
                                provider_str,
                                turn_index,
                                None,
                                &fp,
                                &block_key,
                            ));
                        }
                        _ => {
                            // Unknown item type — preserve as JSON
                            let content = serde_json::to_string_pretty(item).unwrap_or_default();
                            let fp = content_fingerprint(&content);
                            let occ = tracker.next(Role::User, &fp);
                            let block_key = format!("openai:responses:unknown:{occ}");
                            blocks.push(make_block(
                                Role::User,
                                content,
                                provider_str,
                                turn_index,
                                &fp,
                                &block_key,
                            ));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let thread_identity = derive_thread_identity(&raw, &blocks);

    Ok(ParsedRequest {
        provider: Provider::OpenAI,
        model: req.model,
        thread_identity,
        blocks,
        system_prompt: req.instructions,
        stream,
        overhead_tokens,
    })
}

/// Parse an OpenAI Responses API response body.
pub fn parse_openai_responses_response(body: &[u8]) -> Result<ParsedResponse, String> {
    let resp: OpenAIResponsesResponse = serde_json::from_slice(body)
        .map_err(|e| format!("Invalid OpenAI responses response JSON: {e}"))?;

    let provider_str = "openai";
    let mut blocks = Vec::new();
    let mut tracker = OccurrenceTracker::new();

    for output in resp.output.iter() {
        match output.output_type.as_str() {
            "message" => {
                if let Some(content_items) = &output.content {
                    for item in content_items.iter() {
                        if item.content_type == "output_text" || item.content_type == "text" {
                            if let Some(text) = &item.text {
                                let fp = content_fingerprint(text);
                                let occ = tracker.next(Role::Assistant, &fp);
                                let block_key = format!("openai:responses:response:text:{occ}");
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
                let call_id = output.call_id.as_deref().unwrap_or("");
                let fp = content_fingerprint(&content);
                let occ = tracker.next(Role::ToolUse, &fp);
                let block_key = format!("openai:responses:response:function_call:{call_id}:{occ}");
                blocks.push(make_tool_block(
                    Role::ToolUse,
                    content,
                    provider_str,
                    0,
                    output.name.clone(),
                    &fp,
                    &block_key,
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
