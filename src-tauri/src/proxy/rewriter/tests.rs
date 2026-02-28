use super::payload::{
    apply_decisions_to_json, remove_anthropic_messages, remove_openai_chat_messages,
    remove_openai_responses_items, replace_anthropic_content, replace_openai_chat_content,
    replace_openai_responses_content,
};
use super::*;
use crate::engine::planner::applicator::{PartialTurnStub, RewriteDecisions};
use crate::engine::planner::file_tracker::FileMutationKind;
use serde_json::json;
use std::collections::HashSet;

// ── Planner signal extraction tests ──────────────────────

#[test]
fn test_collect_traffic_signals_responses_tracks_current_turn_files_and_mutations() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-4.1",
        "input": [
            {
                "type": "function_call",
                "name": "edit_file",
                "call_id": "old_call",
                "arguments": "{\"path\":\"src/old.rs\"}"
            },
            {
                "type": "function_call_output",
                "call_id": "old_call",
                "output": "old result"
            },
            {
                "type": "function_call",
                "name": "edit_file",
                "call_id": "latest_call",
                "arguments": "{\"path\":\"src/auth.rs\"}"
            },
            {
                "type": "function_call_output",
                "call_id": "latest_call",
                "output": "fn login() { updated }"
            }
        ]
    }))
    .expect("serialize request");

    let signals = collect_traffic_signals("/v1/responses", &body);
    assert_eq!(signals.current_turn_files, vec!["src/auth.rs".to_string()]);
    assert_eq!(signals.file_mutations.len(), 1);
    assert_eq!(signals.file_mutations[0].file_path, "src/auth.rs");
    assert_eq!(signals.file_mutations[0].kind, FileMutationKind::Edit);
}

#[test]
fn test_collect_traffic_signals_chat_tracks_latest_tool_turn_only() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-4.1",
        "messages": [
            {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "write_file",
                        "arguments": "{\"path\":\"src/old.rs\",\"content\":\"old\"}"
                    }
                }]
            },
            {
                "role": "tool",
                "tool_call_id": "call_1",
                "content": "old"
            },
            {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_2",
                    "type": "function",
                    "function": {
                        "name": "write_file",
                        "arguments": "{\"path\":\"src/new.rs\",\"content\":\"new\"}"
                    }
                }]
            },
            {
                "role": "tool",
                "tool_call_id": "call_2",
                "content": "new"
            }
        ]
    }))
    .expect("serialize request");

    let signals = collect_traffic_signals("/v1/chat/completions", &body);
    assert_eq!(signals.current_turn_files, vec!["src/new.rs".to_string()]);
    assert_eq!(signals.file_mutations.len(), 1);
    assert_eq!(signals.file_mutations[0].kind, FileMutationKind::Write);
}

// ── Anthropic format tests ────────────────────────────────────

#[test]
fn test_anthropic_remove_middle_message() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "system": "You are helpful.",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there"},
            {"role": "user", "content": "How are you?"}
        ]
    });

    // Turn 2 = messages[1] (assistant message)
    let turns = HashSet::from([2u32]);
    remove_anthropic_messages(&mut json, &turns);

    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"], "Hello");
    assert_eq!(messages[1]["content"], "How are you?");
}

#[test]
fn test_anthropic_replace_content_string() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {"role": "user", "content": "Old content"},
            {"role": "assistant", "content": "Response"}
        ]
    });

    replace_anthropic_content(&mut json, 1, "New content");

    assert_eq!(json["messages"][0]["content"], "New content");
}

#[test]
fn test_anthropic_replace_content_array() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {"role": "user", "content": [
                {"type": "text", "text": "Old content"}
            ]}
        ]
    });

    replace_anthropic_content(&mut json, 1, "New content");

    assert_eq!(json["messages"][0]["content"][0]["text"], "New content");
}

#[test]
fn test_anthropic_replace_system() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "system": "Old system prompt",
        "messages": []
    });

    replace_anthropic_content(&mut json, 0, "New system prompt");
    assert_eq!(json["system"], "New system prompt");
}

#[test]
fn test_anthropic_remove_system_is_refused() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "system": "System prompt",
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    });

    let turns = HashSet::from([0u32]);
    remove_anthropic_messages(&mut json, &turns);

    // System prompt must NEVER be removed — it would break the API request.
    assert!(json.get("system").is_some());
    assert_eq!(json["system"], "System prompt");
    assert_eq!(json["messages"].as_array().unwrap().len(), 1);
}

// ── OpenAI Chat format tests ──────────────────────────────────

#[test]
fn test_openai_chat_remove_message() {
    let mut json = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "You are helpful"},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"}
        ]
    });

    // Remove turn 1 (user message)
    let turns = HashSet::from([1u32]);
    remove_openai_chat_messages(&mut json, &turns);

    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["role"], "assistant");
}

#[test]
fn test_openai_chat_replace_content() {
    let mut json = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "System"},
            {"role": "user", "content": "Old"}
        ]
    });

    replace_openai_chat_content(&mut json, 1, "New user message");

    assert_eq!(json["messages"][1]["content"], "New user message");
}

// ── OpenAI Responses format tests ─────────────────────────────

#[test]
fn test_responses_remove_item() {
    let mut json = json!({
        "model": "gpt-4",
        "instructions": "Be helpful",
        "input": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"},
            {"role": "user", "content": "Bye"}
        ]
    });

    // Turn 2 = input[1] (assistant)
    let turns = HashSet::from([2u32]);
    remove_openai_responses_items(&mut json, &turns);

    let input = json["input"].as_array().unwrap();
    assert_eq!(input.len(), 2);
    assert_eq!(input[0]["content"], "Hello");
    assert_eq!(input[1]["content"], "Bye");
}

#[test]
fn test_responses_remove_instructions_is_refused() {
    let mut json = json!({
        "model": "gpt-4",
        "instructions": "Be helpful",
        "input": [
            {"role": "user", "content": "Hello"}
        ]
    });

    let turns = HashSet::from([0u32]);
    remove_openai_responses_items(&mut json, &turns);

    // Instructions must NEVER be removed — it would break the API request.
    assert!(json.get("instructions").is_some());
    assert_eq!(json["instructions"], "Be helpful");
    assert_eq!(json["input"].as_array().unwrap().len(), 1);
}

#[test]
fn test_responses_replace_content() {
    let mut json = json!({
        "model": "gpt-4",
        "input": [
            {"role": "user", "content": "Old message"}
        ]
    });

    replace_openai_responses_content(&mut json, 1, "New message");
    assert_eq!(json["input"][0]["content"], "New message");
}

#[test]
fn test_responses_replace_instructions() {
    let mut json = json!({
        "model": "gpt-4",
        "instructions": "Old instructions",
        "input": []
    });

    replace_openai_responses_content(&mut json, 0, "New instructions");
    assert_eq!(json["instructions"], "New instructions");
}

// ── apply_decisions_to_json dispatch tests ────────────────────

#[test]
fn test_apply_decisions_anthropic_path() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"},
            {"role": "user", "content": "Bye"}
        ]
    });

    let mut decisions = RewriteDecisions::default();
    decisions.remove_turns.insert(2); // Remove assistant
    decisions
        .content_replacements
        .insert(3, "Goodbye".to_string()); // Replace last user msg

    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"], "Hello");
    // After removal of index 1, the "Bye" message shifted — but replacement uses original turn index
    // Turn 3 = messages[2] originally, but after removal it's messages[1]
    // The replacement happens BEFORE removal in the current implementation order,
    // so turn 3 = messages[2] is replaced, then turn 2 = messages[1] is removed
}

#[test]
fn test_apply_decisions_openai_chat_path() {
    let mut json = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "System"},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"}
        ]
    });

    let mut decisions = RewriteDecisions::default();
    decisions.remove_turns.insert(1); // Remove user message

    apply_decisions_to_json(&mut json, "/v1/chat/completions", &decisions);

    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
}

#[test]
fn test_apply_decisions_responses_path() {
    let mut json = json!({
        "model": "gpt-4",
        "instructions": "Be helpful",
        "input": [
            {"role": "user", "content": "Hello"}
        ]
    });

    let mut decisions = RewriteDecisions::default();
    decisions
        .content_replacements
        .insert(0, "Updated instructions".to_string());

    apply_decisions_to_json(&mut json, "/v1/responses", &decisions);
    assert_eq!(json["instructions"], "Updated instructions");
}

// ── Edge cases ────────────────────────────────────────────────

#[test]
fn test_remove_multiple_turns_preserves_order() {
    let mut json = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "S"},
            {"role": "user", "content": "U1"},
            {"role": "assistant", "content": "A1"},
            {"role": "user", "content": "U2"},
            {"role": "assistant", "content": "A2"}
        ]
    });

    // Remove turns 1 and 3 (user messages)
    let turns = HashSet::from([1u32, 3u32]);
    remove_openai_chat_messages(&mut json, &turns);

    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["content"], "S");
    assert_eq!(messages[1]["content"], "A1");
    assert_eq!(messages[2]["content"], "A2");
}

#[test]
fn test_empty_turns_no_changes() {
    let mut json = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    });

    let original = json.clone();
    remove_openai_chat_messages(&mut json, &HashSet::new());
    assert_eq!(json, original);
}

#[test]
fn test_replace_content_with_array_input_text() {
    // OpenAI Responses uses "input_text" type
    let mut json = json!({
        "model": "gpt-4",
        "input": [
            {"role": "user", "content": [
                {"type": "input_text", "text": "Old"}
            ]}
        ]
    });

    replace_openai_responses_content(&mut json, 1, "New");
    assert_eq!(json["input"][0]["content"][0]["text"], "New");
}

#[test]
fn test_out_of_bounds_turn_index_ignored() {
    let mut json = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    });

    let turns = HashSet::from([99u32]);
    remove_openai_chat_messages(&mut json, &turns);
    // Should not crash
    assert_eq!(json["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn test_replace_out_of_bounds_turn_index_ignored() {
    let mut json = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "Hello"}
        ]
    });

    replace_openai_chat_content(&mut json, 99, "New");
    // Should not crash, original unchanged
    assert_eq!(json["messages"][0]["content"], "Hello");
}

// ── Tool injection gating tests ──────────────────────────

#[test]
fn test_should_inject_tools_empty_context() {
    let blocks: Vec<crate::engine::block::Block> = vec![];
    assert!(!should_inject_tools(&blocks));
}

#[test]
fn test_should_inject_tools_small_context() {
    use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

    let make = |role: Role| Block {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        block_type: None,
        content: "test".to_string(),
        tokens: 10,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        zone: Zone::BuiltIn(BuiltInZone::Recency),
        pinned: None,
        compression_level: CompressionLevel::Original,
        compressed_versions: CompressionVersions {
            original: CompressionVersion {
                content: "test".to_string(),
                tokens: 10,
            },
            trimmed: None,
            summarized: None,
            minimal: None,
        },
        usage_heat: 0.0,
        position_relevance: 0.0,
        last_referenced_turn: 0,
        reference_count: 0,
        topic_cluster: None,
        topic_keywords: vec![],
        metadata: BlockMetadata {
            provider: "test".to_string(),
            turn_index: 0,
            tool_name: None,
            file_paths: vec![],
        },
    };

    // 1 system + 2 non-system = 2 non-system, not enough
    let blocks = vec![make(Role::System), make(Role::User), make(Role::Assistant)];
    assert!(!should_inject_tools(&blocks));

    // 1 system + 4 non-system = 4 non-system, enough (> 3)
    let blocks = vec![
        make(Role::System),
        make(Role::User),
        make(Role::Assistant),
        make(Role::User),
        make(Role::Assistant),
    ];
    assert!(should_inject_tools(&blocks));
}

// ── Partial-turn stub rewriter tests ──────────────────────────

#[test]
fn test_partial_turn_stubs_anthropic_replaces_tool_result_content() {
    // Anthropic user message with 3 tool_result blocks — archive the 2nd one.
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {"role": "user", "content": "Hello"},
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Reading files..."},
                    {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {"path": "a.rs"}},
                    {"type": "tool_use", "id": "toolu_2", "name": "read_file", "input": {"path": "b.rs"}},
                    {"type": "tool_use", "id": "toolu_3", "name": "read_file", "input": {"path": "c.rs"}}
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "fn main() { println!(\"hello\"); }"},
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": "A very long file content that would be many thousands of tokens in practice..."},
                    {"type": "tool_result", "tool_use_id": "toolu_3", "content": "mod tests { }"}
                ]
            }
        ]
    });

    let original_len = json.to_string().len();

    let mut decisions = RewriteDecisions::default();
    // Archive the 2nd tool_result (index 1 within the user message content array)
    // Turn 3 = messages[2] in Anthropic (turn 0 = system)
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 3,
        content_index: 1,
        stub: "[archived: 5.0k tokens]".to_string(),
    });

    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let new_len = json.to_string().len();
    assert!(
        new_len < original_len,
        "Payload should shrink after stub replacement: {} -> {}",
        original_len,
        new_len
    );

    // Verify the 2nd tool_result has stub content
    let content = json["messages"][2]["content"].as_array().unwrap();
    assert_eq!(content.len(), 3, "All 3 content blocks should still exist");
    assert_eq!(
        content[1]["content"], "[archived: 5.0k tokens]",
        "Archived tool_result content should be replaced with stub"
    );
    // Other blocks unchanged
    assert!(content[0]["content"]
        .as_str()
        .unwrap()
        .contains("fn main"));
    assert!(content[2]["content"].as_str().unwrap().contains("mod tests"));
}

#[test]
fn test_partial_turn_stubs_anthropic_replaces_text_block() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "A very long assistant response that discusses many topics..."},
                    {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {"path": "x.rs"}}
                ]
            }
        ]
    });

    let mut decisions = RewriteDecisions::default();
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 1, // messages[0]
        content_index: 0,
        stub: "[archived: 2.5k tokens]".to_string(),
    });

    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let content = json["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content[0]["text"], "[archived: 2.5k tokens]");
    // tool_use block unchanged
    assert_eq!(content[1]["name"], "read_file");
}

#[test]
fn test_partial_turn_stubs_anthropic_replaces_tool_use_input() {
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "I'll read those files."},
                    {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {"path": "big.rs"}},
                    {"type": "tool_use", "id": "toolu_2", "name": "read_file", "input": {"path": "small.rs"}}
                ]
            }
        ]
    });

    let mut decisions = RewriteDecisions::default();
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 1, // messages[0]
        content_index: 1,
        stub: "[archived: 8.0k tokens]".to_string(),
    });

    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let content = json["messages"][0]["content"].as_array().unwrap();
    // tool_use input replaced with _archived stub
    assert_eq!(
        content[1]["input"]["_archived"],
        "[archived: 8.0k tokens]",
        "tool_use input should be replaced with archived stub"
    );
    // Other blocks unchanged
    assert_eq!(content[0]["text"], "I'll read those files.");
    assert_eq!(content[2]["input"]["path"], "small.rs");
}

#[test]
fn test_partial_turn_stubs_openai_chat_replaces_content() {
    let mut json = json!({
        "model": "gpt-4.1",
        "messages": [
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "A very long user message with lots of context..."},
            {"role": "assistant", "content": "Response"}
        ]
    });

    let mut decisions = RewriteDecisions::default();
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 1, // messages[1] (user message)
        content_index: 0,
        stub: "[archived: 3.0k tokens]".to_string(),
    });

    apply_decisions_to_json(&mut json, "/v1/chat/completions", &decisions);

    assert_eq!(
        json["messages"][1]["content"], "[archived: 3.0k tokens]",
        "OpenAI Chat message content should be replaced with stub"
    );
    // Other messages unchanged
    assert_eq!(json["messages"][0]["content"], "You are helpful.");
    assert_eq!(json["messages"][2]["content"], "Response");
}

#[test]
fn test_partial_turn_stubs_openai_responses_replaces_output() {
    let mut json = json!({
        "model": "gpt-4.1",
        "instructions": "Be helpful",
        "input": [
            {"type": "message", "role": "user", "content": "Hello"},
            {"type": "message", "role": "assistant", "output": "A very long response with detailed analysis..."},
            {"type": "message", "role": "user", "content": "Follow up"}
        ]
    });

    let mut decisions = RewriteDecisions::default();
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 2, // input[1] (assistant message, turn 2 maps to input[1])
        content_index: 0,
        stub: "[archived: 4.0k tokens]".to_string(),
    });

    apply_decisions_to_json(&mut json, "/v1/responses", &decisions);

    assert_eq!(
        json["input"][1]["output"], "[archived: 4.0k tokens]",
        "OpenAI Responses output should be replaced with stub"
    );
    // Other items unchanged
    assert_eq!(json["input"][0]["content"], "Hello");
    assert_eq!(json["input"][2]["content"], "Follow up");
}

// ── End-to-End: Applicator → Payload Pipeline ─────────────────

#[test]
fn test_e2e_partial_turn_archive_anthropic() {
    use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::planner::applicator::apply_mutations;
    use crate::engine::planner::types::ContextMutation;
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

    fn make_block(id: &str, role: Role, turn_index: u32, tokens: u32) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: format!("Content of {id}"),
            tokens,
            timestamp: "2026-02-19T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: format!("Content of {id}"),
                    tokens,
                },
                trimmed: None,
                summarized: None,
                minimal: None,
            },
            usage_heat: 0.0,
            position_relevance: 0.0,
            last_referenced_turn: 0,
            reference_count: 0,
            topic_cluster: None,
            topic_keywords: vec![],
            metadata: BlockMetadata {
                provider: "anthropic".to_string(),
                turn_index,
                tool_name: None,
                file_paths: vec![],
            },
        }
    }

    // Simulate an Anthropic conversation with a multi-block assistant turn:
    // Turn 1: user "Hello"
    // Turn 2: assistant [text + tool_use + tool_use] — 3 blocks
    // Turn 3: user [tool_result + tool_result + tool_result] — 3 blocks
    // Archive 2 of 3 tool_results at turn 3 (partial turn).

    let blocks = vec![
        make_block("u1", Role::User, 1, 50),
        make_block("a1_text", Role::Assistant, 2, 200),
        make_block("a1_tu1", Role::ToolUse, 2, 30),
        make_block("a1_tu2", Role::ToolUse, 2, 30),
        make_block("u2_tr1", Role::ToolResult, 3, 5000),
        make_block("u2_tr2", Role::ToolResult, 3, 8000),
        make_block("u2_tr3", Role::ToolResult, 3, 100),
    ];

    let mutations = vec![
        ContextMutation::Archive {
            block_id: "u2_tr1".into(),
        },
        ContextMutation::Archive {
            block_id: "u2_tr2".into(),
        },
    ];

    // Step 1: Applicator produces decisions
    let decisions = apply_mutations(&blocks, &blocks, &mutations);
    assert!(decisions.remove_turns.is_empty(), "Partial turn — no removal");
    assert_eq!(decisions.partial_turn_stubs.len(), 2, "2 stubs for archived blocks");
    assert!(decisions.has_payload_changes());

    // Step 2: Build a realistic Anthropic JSON payload
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {"role": "user", "content": "Hello"},
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Reading files..."},
                    {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {"path": "a.rs"}},
                    {"type": "tool_use", "id": "toolu_2", "name": "read_file", "input": {"path": "b.rs"}}
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "fn main() { very long content representing 5000 tokens... }"},
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": "mod auth { even longer content representing 8000 tokens... }"},
                    {"type": "tool_result", "tool_use_id": "toolu_3", "content": "mod tests { }"}
                ]
            }
        ]
    });

    let original_len = json.to_string().len();

    // Step 3: Apply decisions to JSON
    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let new_len = json.to_string().len();
    assert!(new_len < original_len, "Payload should shrink: {} -> {}", original_len, new_len);

    // Verify structure preserved
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "All 3 messages preserved (no turn removal)");

    // Turn 3 (messages[2]): tool_results
    let content = messages[2]["content"].as_array().unwrap();
    assert_eq!(content.len(), 3, "All 3 tool_result blocks preserved");
    assert!(content[0]["content"].as_str().unwrap().contains("archived"));
    assert!(content[1]["content"].as_str().unwrap().contains("archived"));
    assert_eq!(content[2]["content"], "mod tests { }", "Unarchived block unchanged");
}

#[test]
fn test_e2e_full_turn_archive_anthropic() {
    use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::planner::applicator::apply_mutations;
    use crate::engine::planner::types::ContextMutation;
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

    fn make_block(id: &str, role: Role, turn_index: u32, tokens: u32) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: format!("Content of {id}"),
            tokens,
            timestamp: "2026-02-19T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: format!("Content of {id}"),
                    tokens,
                },
                trimmed: None,
                summarized: None,
                minimal: None,
            },
            usage_heat: 0.0,
            position_relevance: 0.0,
            last_referenced_turn: 0,
            reference_count: 0,
            topic_cluster: None,
            topic_keywords: vec![],
            metadata: BlockMetadata {
                provider: "anthropic".to_string(),
                turn_index,
                tool_name: None,
                file_paths: vec![],
            },
        }
    }

    // Turn 1: user "Hello"
    // Turn 2: assistant "Long response" — single block, archive ALL → full turn removal
    // Turn 3: user "Follow up"
    let blocks = vec![
        make_block("u1", Role::User, 1, 50),
        make_block("a1", Role::Assistant, 2, 5000),
        make_block("u2", Role::User, 3, 80),
    ];

    let mutations = vec![ContextMutation::Archive {
        block_id: "a1".into(),
    }];

    let decisions = apply_mutations(&blocks, &blocks, &mutations);
    assert!(decisions.remove_turns.contains(&2), "Full-turn archive → turn removal");
    assert!(decisions.partial_turn_stubs.is_empty(), "No stubs for full-turn removal");

    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "A very long response..."},
            {"role": "user", "content": "Follow up"}
        ]
    });

    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "Full-turn archive removes the message");
    assert_eq!(messages[0]["content"], "Hello");
    assert_eq!(messages[1]["content"], "Follow up");
}

// ── Pipeline ordering tests ──────────────────────────────────

#[test]
fn test_stubs_applied_before_removal_so_indices_are_correct() {
    // Scenario: Turn 2 (messages[1]) is fully removed, Turn 3 (messages[2]) has partial stubs.
    // If removal happened first, messages[2] would shift to messages[1] and stubs targeting
    // turn_index=3 (messages[2]) would miss.
    let mut json = json!({
        "model": "claude-sonnet-4-5-20250929",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Old response to remove"},
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "Big result to stub"},
                    {"type": "tool_result", "tool_use_id": "t2", "content": "Keep this one"}
                ]
            },
            {"role": "assistant", "content": "Latest response"}
        ]
    });

    let mut decisions = RewriteDecisions::default();
    // Full-turn removal of turn 2 (messages[1])
    decisions.remove_turns.insert(2);
    // Partial stub on turn 3 (messages[2]), content index 0
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 3,
        content_index: 0,
        stub: "[archived: 5.0k tokens]".to_string(),
    });

    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let messages = json["messages"].as_array().unwrap();
    // After removal of messages[1], we should have 3 messages left
    assert_eq!(messages.len(), 3);

    // The stub should have landed on what was originally messages[2] (now messages[1])
    let content = messages[1]["content"].as_array().unwrap();
    assert_eq!(
        content[0]["content"], "[archived: 5.0k tokens]",
        "Stub should be applied to the correct message before removal shifts indices"
    );
    assert_eq!(
        content[1]["content"], "Keep this one",
        "Non-archived block should be unchanged"
    );
}

// ── Thinking block protection tests ───────────────────────────

#[test]
fn test_partial_turn_stubs_skip_thinking_and_redacted_thinking_blocks() {
    let mut json = json!({
        "model": "claude-opus-4-6-20260219",
        "messages": [
            {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "Deep analysis of the problem...", "signature": "sig123"},
                    {"type": "text", "text": "Here is my analysis."},
                    {"type": "redacted_thinking", "data": "abc123"}
                ]
            }
        ]
    });

    let original = json.clone();

    let mut decisions = RewriteDecisions::default();
    // Try to stub-replace both thinking blocks (indices 0 and 2)
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 1,
        content_index: 0,
        stub: "[archived: 1.0k tokens]".to_string(),
    });
    decisions.partial_turn_stubs.push(PartialTurnStub {
        turn_index: 1,
        content_index: 2,
        stub: "[archived: 0.5k tokens]".to_string(),
    });

    apply_decisions_to_json(&mut json, "/v1/messages", &decisions);

    let content = json["messages"][0]["content"].as_array().unwrap();
    // Thinking blocks must be byte-identical to original
    assert_eq!(
        content[0], original["messages"][0]["content"][0],
        "thinking block must not be modified"
    );
    assert_eq!(
        content[2], original["messages"][0]["content"][2],
        "redacted_thinking block must not be modified"
    );
    // Text block unchanged (not targeted)
    assert_eq!(content[1]["text"], "Here is my analysis.");
}

// ── Message structure sanitizer tests ──────────────────────────

#[test]
fn test_structure_sanitizer_noop_well_formed() {
    use super::sanitize::sanitize_anthropic_message_structure;

    let mut json = json!({
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "Hello"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "Hi"}]},
            {"role": "user", "content": [{"type": "text", "text": "Bye"}]}
        ]
    });
    let original = json.clone();
    let changes = sanitize_anthropic_message_structure(&mut json);
    assert_eq!(changes, 0);
    assert_eq!(json, original);
}

#[test]
fn test_structure_sanitizer_merges_two_consecutive_assistants() {
    use super::sanitize::sanitize_anthropic_message_structure;

    let mut json = json!({
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "Hello"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "Part 1"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "Part 2"}]},
            {"role": "user", "content": [{"type": "text", "text": "Ok"}]}
        ]
    });
    let changes = sanitize_anthropic_message_structure(&mut json);
    assert!(changes >= 1, "Should merge at least once");
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    // Merged content should have both blocks
    let content = messages[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["text"], "Part 1");
    assert_eq!(content[1]["text"], "Part 2");
    assert_eq!(messages[2]["role"], "user");
}

#[test]
fn test_structure_sanitizer_merges_three_consecutive_assistants() {
    use super::sanitize::sanitize_anthropic_message_structure;

    let mut json = json!({
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "Hello"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "A"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "B"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "C"}]},
            {"role": "user", "content": [{"type": "text", "text": "Ok"}]}
        ]
    });
    let changes = sanitize_anthropic_message_structure(&mut json);
    assert_eq!(changes, 2, "Two merges for three consecutive assistants");
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    let content = messages[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 3);
}

#[test]
fn test_structure_sanitizer_appends_user_when_last_is_assistant() {
    use super::sanitize::sanitize_anthropic_message_structure;

    let mut json = json!({
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "Hello"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "Thinking..."}]}
        ]
    });
    let changes = sanitize_anthropic_message_structure(&mut json);
    assert_eq!(changes, 1, "One synthetic user message appended");
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["role"], "user");
    let content = messages[2]["content"].as_array().unwrap();
    assert_eq!(content[0]["text"], "Continue.");
}

#[test]
fn test_structure_sanitizer_full_cleanup_scenario() {
    use super::sanitize::sanitize_anthropic_message_structure;

    // Simulates the state AFTER strip_anthropic_context_tools() has run:
    // Original: [user, assistant+tool_use, user(tool_result_only), assistant+tool_use, user(tool_result_only)]
    // After cleanup: user messages with only tool_results removed → [user, assistant, assistant]
    let mut json = json!({
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "clear context"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "Let me check..."}]},
            {"role": "assistant", "content": [{"type": "text", "text": "I'll archive..."}]}
        ]
    });
    let changes = sanitize_anthropic_message_structure(&mut json);
    // 1 merge (2 assistants → 1) + 1 synthetic user = 2 changes
    assert_eq!(changes, 2);
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    let content = messages[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "Both assistant texts merged");
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"][0]["text"], "Continue.");
}

#[test]
fn test_structure_sanitizer_preserves_thinking_blocks_during_merge() {
    use super::sanitize::sanitize_anthropic_message_structure;

    // Consecutive assistant messages where one has thinking blocks should NOT be merged.
    // Instead, a synthetic user message is inserted between them.
    let mut json = json!({
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "Hello"}]},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "Deep analysis...", "signature": "sig1"},
                {"type": "text", "text": "Part 1"}
            ]},
            {"role": "assistant", "content": [{"type": "text", "text": "Part 2"}]},
            {"role": "user", "content": [{"type": "text", "text": "Ok"}]}
        ]
    });
    let changes = sanitize_anthropic_message_structure(&mut json);
    assert!(changes >= 1, "Should insert synthetic user message");
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5, "user, assistant(thinking), synthetic user, assistant, user");
    // Thinking assistant stays separate
    assert_eq!(messages[1]["role"], "assistant");
    let content = messages[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "thinking + text, NOT merged");
    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["thinking"], "Deep analysis...");
    assert_eq!(content[1]["text"], "Part 1");
    // Synthetic user separates them
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"][0]["text"], "Continue.");
    // Second assistant unchanged
    assert_eq!(messages[3]["role"], "assistant");
    assert_eq!(messages[3]["content"][0]["text"], "Part 2");
    // Original user at end
    assert_eq!(messages[4]["role"], "user");
    assert_eq!(messages[4]["content"][0]["text"], "Ok");
}

#[test]
fn test_structure_sanitizer_noop_for_no_messages() {
    use super::sanitize::sanitize_anthropic_message_structure;

    // OpenAI paths don't have "messages" in the same shape, but more importantly
    // the function should be a no-op when there are no messages.
    let mut json = json!({"input": [{"role": "user", "content": "Hello"}]});
    let changes = sanitize_anthropic_message_structure(&mut json);
    assert_eq!(changes, 0);
}
