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

#[test]
fn test_anthropic_request_ids_stable_across_identical_parses() {
    let body = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": "one"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_123", "name": "grep", "input": {"pattern": "one"}},
                {"type": "text", "text": "running grep"}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_123", "content": "found"}
            ]}
        ]
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();

    let first = parse_anthropic_request(&body_bytes).unwrap();
    let second = parse_anthropic_request(&body_bytes).unwrap();
    let first_ids: Vec<&str> = first.blocks.iter().map(|b| b.id.as_str()).collect();
    let second_ids: Vec<&str> = second.blocks.iter().map(|b| b.id.as_str()).collect();

    assert_eq!(first_ids, second_ids);
}

#[test]
fn test_openai_chat_request_prefix_ids_stable_when_appending_turns() {
    let base = serde_json::json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "Hello"}
        ]
    });
    let extended = serde_json::json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there"}
        ]
    });

    let base_result = parse_openai_chat_request(&serde_json::to_vec(&base).unwrap()).unwrap();
    let extended_result =
        parse_openai_chat_request(&serde_json::to_vec(&extended).unwrap()).unwrap();

    assert!(base_result.blocks.len() <= extended_result.blocks.len());
    for (idx, block) in base_result.blocks.iter().enumerate() {
        assert_eq!(
            block.id, extended_result.blocks[idx].id,
            "expected stable ID at prefix index {idx}"
        );
    }
}

// --- Overhead token estimation ---

#[test]
fn test_anthropic_request_with_tools_has_overhead() {
    let body = serde_json::json!({
        "model": "claude-opus-4-6",
        "max_tokens": 1024,
        "tools": [
            {
                "name": "Read",
                "description": "Read a file from disk",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "file_path": {"type": "string", "description": "Path to read"}
                    },
                    "required": ["file_path"]
                }
            },
            {
                "name": "Bash",
                "description": "Execute a bash command",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"}
                    },
                    "required": ["command"]
                }
            }
        ],
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let result = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    // Two tool definitions with schemas → should be > 0
    assert!(
        result.overhead_tokens > 0,
        "overhead_tokens should be > 0 when tools array present, got {}",
        result.overhead_tokens
    );
}

#[test]
fn test_request_without_tools_has_zero_overhead() {
    let body = serde_json::json!({
        "model": "claude-opus-4-6",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let result = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert_eq!(result.overhead_tokens, 0);
}

#[test]
fn test_openai_chat_request_with_tools_has_overhead() {
    let body = serde_json::json!({
        "model": "gpt-4",
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "parameters": {"type": "object", "properties": {"loc": {"type": "string"}}}
                }
            }
        ],
        "messages": [{"role": "user", "content": "weather?"}]
    });
    let result = parse_openai_chat_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(result.overhead_tokens > 0);
}

#[test]
fn test_openai_responses_request_with_tools_has_overhead() {
    let body = serde_json::json!({
        "model": "gpt-4",
        "tools": [
            {
                "type": "function",
                "name": "search",
                "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
            }
        ],
        "input": "search for cats"
    });
    let result = parse_openai_responses_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(result.overhead_tokens > 0);
}

#[test]
fn test_thread_identity_uses_explicit_thread_id_when_present() {
    let body = serde_json::json!({
        "model": "gpt-4.1",
        "thread_id": "thread-abc-123",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let result = parse_openai_chat_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert_eq!(result.thread_identity.as_deref(), Some("thread-abc-123"));
}

/// With fallback hashing disabled, requests without explicit thread IDs
/// should produce None thread_identity (regardless of message content).
#[test]
fn test_thread_identity_none_without_explicit_ids() {
    let body = serde_json::json!({
        "model": "claude-sonnet-4-5",
        "system": "You are helpful.",
        "messages": [
            {"role": "user", "content": "Implement foo"},
            {"role": "assistant", "content": "Working on it"}
        ]
    });

    let parsed = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(
        parsed.thread_identity.is_none(),
        "Without explicit thread IDs, thread_identity must be None"
    );
}

/// Billing header churn no longer matters — no fallback hashing at all.
#[test]
fn test_thread_identity_none_regardless_of_system_content() {
    let body = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "system": "x-anthropic-billing-header: cc_version=2.1.47.b96; cch=abc123;\nYou are helpful.",
        "messages": [
            {"role": "user", "content": "Howdy claude"},
            {"role": "assistant", "content": "Hey! What are we building today?"}
        ]
    });

    let parsed = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(
        parsed.thread_identity.is_none(),
        "Without explicit thread IDs, thread_identity must be None"
    );
}

/// System-reminder blocks no longer affect identity — fallback disabled.
#[test]
fn test_thread_identity_none_with_transient_blocks() {
    let body = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "<system-reminder>\nTransient metadata A"},
            {"role": "user", "content": "Howdy claude"},
            {"role": "assistant", "content": "Hey! What are we building today?"}
        ]
    });

    let parsed = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(
        parsed.thread_identity.is_none(),
        "Without explicit thread IDs, thread_identity must be None"
    );
}

// --- Content-fingerprint block ID stability ---

#[test]
fn test_same_content_at_different_indices_produces_same_id() {
    // A block with content "Hello" at message index 1 should produce the
    // same ID as the same block at message index 5 (position-independent).
    let body_early = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    });
    let body_late = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "assistant", "content": "Preamble 1"},
            {"role": "user", "content": "Filler"},
            {"role": "assistant", "content": "Preamble 2"},
            {"role": "user", "content": "More filler"},
            {"role": "assistant", "content": "Preamble 3"},
            {"role": "user", "content": "Hello"}
        ]
    });

    let early = parse_anthropic_request(&serde_json::to_vec(&body_early).unwrap()).unwrap();
    let late = parse_anthropic_request(&serde_json::to_vec(&body_late).unwrap()).unwrap();

    let early_hello_id = &early.blocks[0].id;
    let late_hello = late.blocks.iter().find(|b| b.content == "Hello").unwrap();

    assert_eq!(
        early_hello_id, &late_hello.id,
        "same content should produce same block ID regardless of position"
    );
}

#[test]
fn test_two_identical_content_blocks_produce_different_ids() {
    // Two user messages with the same content should get different IDs
    // via the occurrence counter.
    let body = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"},
            {"role": "user", "content": "Hello"}
        ]
    });

    let result = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();
    let hello_blocks: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.content == "Hello")
        .collect();

    assert_eq!(hello_blocks.len(), 2);
    assert_ne!(
        hello_blocks[0].id, hello_blocks[1].id,
        "identical content blocks should have different IDs via occurrence counter"
    );
}

#[test]
fn test_removing_middle_message_preserves_surrounding_ids() {
    let body_full = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "First question"},
            {"role": "assistant", "content": "First answer"},
            {"role": "user", "content": "Second question"},
            {"role": "assistant", "content": "Second answer"},
            {"role": "user", "content": "Third question"}
        ]
    });
    // Remove the middle exchange (second question + answer)
    let body_trimmed = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "First question"},
            {"role": "assistant", "content": "First answer"},
            {"role": "user", "content": "Third question"}
        ]
    });

    let full = parse_anthropic_request(&serde_json::to_vec(&body_full).unwrap()).unwrap();
    let trimmed = parse_anthropic_request(&serde_json::to_vec(&body_trimmed).unwrap()).unwrap();

    // First question ID should be stable
    let full_first = full
        .blocks
        .iter()
        .find(|b| b.content == "First question")
        .unwrap();
    let trimmed_first = trimmed
        .blocks
        .iter()
        .find(|b| b.content == "First question")
        .unwrap();
    assert_eq!(full_first.id, trimmed_first.id, "First question ID shifted");

    // First answer ID should be stable
    let full_ans = full
        .blocks
        .iter()
        .find(|b| b.content == "First answer")
        .unwrap();
    let trimmed_ans = trimmed
        .blocks
        .iter()
        .find(|b| b.content == "First answer")
        .unwrap();
    assert_eq!(full_ans.id, trimmed_ans.id, "First answer ID shifted");

    // Third question ID should be stable
    let full_third = full
        .blocks
        .iter()
        .find(|b| b.content == "Third question")
        .unwrap();
    let trimmed_third = trimmed
        .blocks
        .iter()
        .find(|b| b.content == "Third question")
        .unwrap();
    assert_eq!(full_third.id, trimmed_third.id, "Third question ID shifted");
}

#[test]
fn test_adding_message_preserves_existing_ids() {
    let body_base = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "What is Rust?"},
            {"role": "assistant", "content": "A systems language."}
        ]
    });
    let body_extended = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "What is Rust?"},
            {"role": "assistant", "content": "A systems language."},
            {"role": "user", "content": "Tell me more."},
            {"role": "assistant", "content": "It focuses on safety."}
        ]
    });

    let base = parse_anthropic_request(&serde_json::to_vec(&body_base).unwrap()).unwrap();
    let extended = parse_anthropic_request(&serde_json::to_vec(&body_extended).unwrap()).unwrap();

    // All base block IDs should appear unchanged in extended
    for base_block in &base.blocks {
        let ext_match = extended
            .blocks
            .iter()
            .find(|b| b.content == base_block.content)
            .unwrap_or_else(|| panic!("missing block: {}", base_block.content));
        assert_eq!(
            base_block.id, ext_match.id,
            "block '{}' changed ID after appending",
            base_block.content
        );
    }
}

#[test]
fn test_tool_use_blocks_stable_when_preceding_messages_shift() {
    let body_short = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "Read my file"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_stable_001", "name": "Read", "input": {"path": "/tmp/foo"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_stable_001", "content": "file contents here"}
            ]}
        ]
    });
    // Same conversation but with extra messages inserted BEFORE the tool call
    let body_long = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there!"},
            {"role": "user", "content": "What can you do?"},
            {"role": "assistant", "content": "Lots of things."},
            {"role": "user", "content": "Read my file"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_stable_001", "name": "Read", "input": {"path": "/tmp/foo"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_stable_001", "content": "file contents here"}
            ]}
        ]
    });

    let short = parse_anthropic_request(&serde_json::to_vec(&body_short).unwrap()).unwrap();
    let long = parse_anthropic_request(&serde_json::to_vec(&body_long).unwrap()).unwrap();

    // Find tool_use blocks
    let short_tool = short
        .blocks
        .iter()
        .find(|b| b.role == Role::ToolUse)
        .unwrap();
    let long_tool = long
        .blocks
        .iter()
        .find(|b| b.role == Role::ToolUse)
        .unwrap();
    assert_eq!(
        short_tool.id, long_tool.id,
        "tool_use block ID shifted when preceding messages changed"
    );

    // Find tool_result blocks
    let short_result = short
        .blocks
        .iter()
        .find(|b| b.role == Role::ToolResult)
        .unwrap();
    let long_result = long
        .blocks
        .iter()
        .find(|b| b.role == Role::ToolResult)
        .unwrap();
    assert_eq!(
        short_result.id, long_result.id,
        "tool_result block ID shifted when preceding messages changed"
    );
}

// --- estimate_request_overhead tests ---

#[test]
fn test_estimate_request_overhead_includes_system_and_tools() {
    let raw = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "system": "You are a helpful assistant with extensive knowledge.",
        "tools": [
            { "name": "read_file", "description": "Read a file", "input_schema": { "type": "object" } }
        ],
        "messages": [
            { "role": "user", "content": "hello" }
        ]
    });

    let overhead = estimate_request_overhead(&raw);
    let tools_str = serde_json::to_string(raw.get("tools").unwrap()).unwrap();
    let expected_tool_tokens = (tools_str.len() as u32) / 4;
    let system_text = "You are a helpful assistant with extensive knowledge.";
    let expected_system_tokens = (system_text.len() as u32) / 4;

    assert_eq!(
        overhead,
        expected_tool_tokens + expected_system_tokens,
        "overhead should include both tool and system tokens"
    );
    assert!(
        overhead > expected_tool_tokens,
        "system tokens should add to overhead"
    );
    assert!(
        overhead > expected_system_tokens,
        "tool tokens should add to overhead"
    );
}

#[test]
fn test_estimate_request_overhead_no_tools() {
    let raw = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "system": "You are a coding assistant.",
        "messages": [
            { "role": "user", "content": "hello" }
        ]
    });

    let overhead = estimate_request_overhead(&raw);
    let expected = ("You are a coding assistant.".len() as u32) / 4;
    assert_eq!(
        overhead, expected,
        "overhead should be system tokens only when no tools"
    );
}

#[test]
fn test_estimate_request_overhead_no_system() {
    let raw = serde_json::json!({
        "model": "gpt-4",
        "tools": [
            { "name": "search", "description": "Search the web", "input_schema": { "type": "object" } }
        ],
        "messages": [
            { "role": "user", "content": "hello" }
        ]
    });

    let overhead = estimate_request_overhead(&raw);
    let tools_str = serde_json::to_string(raw.get("tools").unwrap()).unwrap();
    let expected = (tools_str.len() as u32) / 4;
    assert_eq!(
        overhead, expected,
        "overhead should be tool tokens only when no system"
    );
}

#[test]
fn test_estimate_request_overhead_anthropic_system_array() {
    let raw = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "system": [
            { "type": "text", "text": "You are helpful." },
            { "type": "text", "text": "Be concise." }
        ],
        "messages": [
            { "role": "user", "content": "hi" }
        ]
    });

    let overhead = estimate_request_overhead(&raw);
    let expected = (("You are helpful.".len() + "Be concise.".len()) as u32) / 4;
    assert_eq!(
        overhead, expected,
        "should sum text from content-block array"
    );
}

#[test]
fn test_estimate_request_overhead_openai_instructions() {
    let raw = serde_json::json!({
        "model": "codex-mini",
        "instructions": "You are a code reviewer.",
        "input": [
            { "role": "user", "content": "review this" }
        ]
    });

    let overhead = estimate_request_overhead(&raw);
    let expected = ("You are a code reviewer.".len() as u32) / 4;
    assert_eq!(
        overhead, expected,
        "should extract system from instructions field"
    );
}

#[test]
fn test_estimate_request_overhead_openai_chat_system_message() {
    let raw = serde_json::json!({
        "model": "gpt-4",
        "messages": [
            { "role": "system", "content": "You are a translator." },
            { "role": "user", "content": "translate this" }
        ]
    });

    let overhead = estimate_request_overhead(&raw);
    let expected = ("You are a translator.".len() as u32) / 4;
    assert_eq!(
        overhead, expected,
        "should extract system from messages[0] with role system"
    );
}

// ── System block fingerprint stability tests ──────────────────

#[test]
fn test_system_fingerprint_stable_despite_billing_header() {
    let body_a = serde_json::to_vec(&serde_json::json!({
        "model": "claude-opus-4-6-20250929",
        "system": "x-anthropic-billing-header: cc_version=2.1.47.b96; cc_entrypoint=cli\nYou are Claude Code, Anthropic's CLI.",
        "messages": [{"role": "user", "content": "hi"}]
    })).unwrap();
    let body_b = serde_json::to_vec(&serde_json::json!({
        "model": "claude-opus-4-6-20250929",
        "system": "x-anthropic-billing-header: cc_version=2.1.48.b99; cc_entrypoint=vscode\nYou are Claude Code, Anthropic's CLI.",
        "messages": [{"role": "user", "content": "hi"}]
    })).unwrap();

    let parsed_a = parse_anthropic_request(&body_a).unwrap();
    let parsed_b = parse_anthropic_request(&body_b).unwrap();

    let sys_a = parsed_a.blocks.iter().find(|b| b.role == Role::System).unwrap();
    let sys_b = parsed_b.blocks.iter().find(|b| b.role == Role::System).unwrap();

    assert_eq!(sys_a.id, sys_b.id, "System blocks with different billing headers should have same ID");
}

#[test]
fn test_system_fingerprint_differs_for_real_content_changes() {
    let body_a = serde_json::to_vec(&serde_json::json!({
        "model": "claude-opus-4-6-20250929",
        "system": "x-anthropic-billing-header: cc_version=2.1.47\nYou are Claude Code.",
        "messages": [{"role": "user", "content": "hi"}]
    })).unwrap();
    let body_b = serde_json::to_vec(&serde_json::json!({
        "model": "claude-opus-4-6-20250929",
        "system": "x-anthropic-billing-header: cc_version=2.1.47\nYou are a helpful assistant.",
        "messages": [{"role": "user", "content": "hi"}]
    })).unwrap();

    let parsed_a = parse_anthropic_request(&body_a).unwrap();
    let parsed_b = parse_anthropic_request(&body_b).unwrap();

    let sys_a = parsed_a.blocks.iter().find(|b| b.role == Role::System).unwrap();
    let sys_b = parsed_b.blocks.iter().find(|b| b.role == Role::System).unwrap();

    assert_ne!(sys_a.id, sys_b.id, "System blocks with different instructions should have different IDs");
}

/// H9 FIX: Without explicit thread IDs, thread_identity is None.
/// Fallback content-hashing was disabled because it produced different hashes
/// on every request (Claude Code injects varying <system-reminder> content),
/// causing catastrophic session fragmentation (20+ sessions for one conversation).
///
/// With the fix, sessions key on (provider, model, source, "default") — one
/// session per provider/model combo, which is correct for single-instance usage.
#[test]
fn test_h9_fix_no_fallback_identity_without_explicit_thread() {
    let body = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "system": "You are Claude Code, Anthropic's official CLI.",
        "messages": [
            {"role": "user", "content": "Help me build a web server in Rust"},
            {"role": "assistant", "content": "I'd be happy to help you build a web server in Rust."},
            {"role": "user", "content": "Use axum for the framework please"},
            {"role": "assistant", "content": "Sure, let me set up axum with tokio."}
        ]
    });

    let parsed = parse_anthropic_request(&serde_json::to_vec(&body).unwrap()).unwrap();

    // No explicit thread_id/session_id/conversation_id in the request →
    // thread_identity should be None (falls through to "default" in session_identity_key).
    assert!(
        parsed.thread_identity.is_none(),
        "Without explicit thread IDs, thread_identity must be None to prevent fragmentation.\n  \
         Got: {:?}",
        parsed.thread_identity
    );
}

/// Codex sends `previous_response_id` which IS an explicit thread identifier.
/// Verify it's still detected correctly after disabling fallback hashing.
#[test]
fn test_explicit_thread_identity_still_works() {
    let body = serde_json::json!({
        "model": "gpt-4.1",
        "previous_response_id": "resp_abc123def456",
        "input": [
            {"role": "user", "content": "Hello"}
        ]
    });

    let parsed = parse_openai_responses_request(
        &serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();

    assert_eq!(
        parsed.thread_identity.as_deref(),
        Some("resp_abc123def456"),
        "Explicit thread IDs (like Codex's previous_response_id) must still be detected"
    );
}

#[test]
fn test_system_fingerprint_unaffected_without_billing_header() {
    let body_a = serde_json::to_vec(&serde_json::json!({
        "model": "claude-opus-4-6-20250929",
        "system": "You are Claude Code, Anthropic's CLI.",
        "messages": [{"role": "user", "content": "hi"}]
    })).unwrap();
    let body_b = serde_json::to_vec(&serde_json::json!({
        "model": "claude-opus-4-6-20250929",
        "system": "You are Claude Code, Anthropic's CLI.",
        "messages": [{"role": "user", "content": "hi"}]
    })).unwrap();

    let parsed_a = parse_anthropic_request(&body_a).unwrap();
    let parsed_b = parse_anthropic_request(&body_b).unwrap();

    let sys_a = parsed_a.blocks.iter().find(|b| b.role == Role::System).unwrap();
    let sys_b = parsed_b.blocks.iter().find(|b| b.role == Role::System).unwrap();

    assert_eq!(sys_a.id, sys_b.id, "Identical system prompts without billing headers should produce same ID");
}
