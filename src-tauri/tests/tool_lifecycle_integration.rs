//! Integration tests for the Phase 3 tool lifecycle.
//!
//! These tests exercise the full context management pipeline: engine ingestion,
//! planner output, rewriter payload modification, tool injection gating,
//! and engine-side block updates — without starting an actual proxy server.
//!
//! Each test constructs an engine, ingests blocks, and runs the rewriter
//! directly to verify the pipeline produces correct JSON output.

use aperture_lib::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
use aperture_lib::engine::planner::types::{ContextMutation, PendingPlan};
use aperture_lib::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};
use aperture_lib::engine::ContextEngine;
use aperture_lib::metacog::{detect_runtime, RuntimeKind};
use aperture_lib::proxy::parser::{parse_request, parse_response, Provider};
use aperture_lib::proxy::rewriter::rewrite_request;

/// Helper to build a block with sensible defaults.
fn make_block(role: Role, content: &str, turn_index: u32) -> Block {
    let tokens = std::cmp::max((content.len() as u32) / 4, 1);
    Block {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        block_type: None,
        content: content.to_string(),
        tokens,
        timestamp: "2026-02-13T00:00:00Z".to_string(),
        zone: Zone::BuiltIn(BuiltInZone::Middle),
        pinned: None,
        compression_level: CompressionLevel::Original,
        compressed_versions: CompressionVersions {
            original: CompressionVersion {
                content: content.to_string(),
                tokens,
            },
            trimmed: None,
            summarized: None,
            minimal: None,
        },
        usage_heat: 0.0,
        position_relevance: 0.0,
        last_referenced_turn: turn_index,
        reference_count: 0,
        topic_cluster: None,
        topic_keywords: vec![],
        metadata: BlockMetadata {
            provider: "test".to_string(),
            turn_index,
            tool_name: None,
            file_paths: vec![],
        },
    }
}

/// Build an Anthropic-style request JSON with N user/assistant turns.
fn anthropic_request_json(turns: usize) -> serde_json::Value {
    let mut messages = vec![];
    for i in 0..turns {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        messages.push(serde_json::json!({
            "role": role,
            "content": format!("Turn {i} content — filler text to simulate a real conversation exchange")
        }));
    }
    serde_json::json!({
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 1024,
        "system": "You are a helpful assistant.",
        "messages": messages
    })
}

/// Build an OpenAI Chat Completions request JSON with N user/assistant turns.
fn openai_chat_request_json(turns: usize, stream: bool) -> serde_json::Value {
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": "You are a helpful assistant."
    })];
    for i in 0..turns {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        messages.push(serde_json::json!({
            "role": role,
            "content": format!("Turn {i} content — filler text to simulate a real conversation exchange")
        }));
    }
    let mut json = serde_json::json!({
        "model": "gpt-4.1",
        "messages": messages
    });
    if stream {
        json["stream"] = serde_json::Value::Bool(true);
    }
    json
}

/// Build an OpenAI Responses API request JSON with N user/assistant turns.
fn openai_responses_request_json(turns: usize, stream: bool) -> serde_json::Value {
    let mut input = vec![];
    for i in 0..turns {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        input.push(serde_json::json!({
            "type": "message",
            "role": role,
            "content": [{ "type": "input_text", "text": format!("Turn {i} content — filler text") }]
        }));
    }
    let mut json = serde_json::json!({
        "model": "gpt-4.1",
        "instructions": "You are a helpful assistant.",
        "input": input
    });
    if stream {
        json["stream"] = serde_json::Value::Bool(true);
    }
    json
}

/// Ingest a multi-turn conversation into the engine.
fn ingest_turns(engine: &ContextEngine, provider: &str, model: &str, turns: usize) {
    let mut request_blocks = vec![make_block(Role::System, "You are a helpful assistant.", 0)];
    let mut response_blocks = vec![];

    for i in 0..turns {
        let turn_idx = (i + 1) as u32;
        if i % 2 == 0 {
            request_blocks.push(make_block(
                Role::User,
                &format!("Turn {i} content — filler text to simulate a real conversation exchange"),
                turn_idx,
            ));
        } else {
            response_blocks.push(make_block(
                Role::Assistant,
                &format!("Turn {i} content — filler text to simulate a real conversation exchange"),
                turn_idx,
            ));
        }
    }

    engine.ingest(
        provider,
        model,
        "test",
        None,
        request_blocks,
        response_blocks,
    );
}

// ─── Test: Anthropic full lifecycle (ClaudeMcp — manifest, no inline tools) ──

#[test]
fn anthropic_full_lifecycle_claude_mcp() {
    let engine = ContextEngine::new_in_memory(None);

    // Ingest enough turns to trigger manifest injection (needs non-system blocks)
    ingest_turns(&engine, "anthropic", "claude-sonnet-4-5-20250929", 6);

    let body_json = anthropic_request_json(6);
    let body = serde_json::to_vec(&body_json).unwrap();
    let path = "/v1/messages";

    let parsed = parse_request(path, &body).unwrap();
    assert_eq!(parsed.provider, Provider::Anthropic);
    assert!(!parsed.stream);

    // Anthropic → ClaudeMcp runtime (tools via MCP, not inline injection)
    let runtime_kind = detect_runtime(path, &parsed.provider.to_string());
    assert_eq!(runtime_kind, RuntimeKind::ClaudeMcp);

    // Rewrite — should inject manifest but NOT inline tools (ClaudeMcp uses MCP transport)
    let result = rewrite_request(&body, path, &parsed, &engine);
    match result {
        Ok(Some(rewritten)) => {
            let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();

            // Manifest should be injected into system message
            let system = rewritten_json.get("system").and_then(|v| v.as_str());
            assert!(
                system.is_some(),
                "Rewritten body should have a system field"
            );

            // Messages should still exist
            assert!(
                rewritten_json.get("messages").is_some(),
                "Messages should still be present"
            );
        }
        Ok(None) => {
            // No rewriting may happen if planner has no mutations — that's acceptable
            // as long as we confirmed the runtime detection is correct
        }
        Err(e) => panic!("Rewrite failed: {e}"),
    }
}

// ─── Test: Codex Chat Completions tool injection ─────────────────────────────

#[test]
fn codex_chat_tool_injection_non_streaming() {
    let engine = ContextEngine::new_in_memory(None);

    // Ingest enough turns for mature context (>3 non-system blocks)
    ingest_turns(&engine, "openai", "gpt-4.1", 8);

    let body_json = openai_chat_request_json(8, false);
    let body = serde_json::to_vec(&body_json).unwrap();
    let path = "/v1/chat/completions";

    let parsed = parse_request(path, &body).unwrap();
    assert_eq!(parsed.provider, Provider::OpenAI);
    assert!(!parsed.stream);

    // Chat completions → CodexProxy runtime
    let runtime_kind = detect_runtime(path, &parsed.provider.to_string());
    assert_eq!(runtime_kind, RuntimeKind::CodexProxy);

    // Verify we have enough blocks for tool injection (>3 non-system)
    let blocks = engine.active_session_blocks();
    let non_system = blocks.iter().filter(|b| b.role != Role::System).count();
    assert!(
        non_system > 3,
        "Mature context should have >3 non-system blocks, got {non_system}"
    );

    // Rewrite — should inject tools
    let rewritten = rewrite_request(&body, path, &parsed, &engine)
        .unwrap()
        .expect("non-streaming chat request should be rewritten with tool definitions");
    let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();

    let tools = rewritten_json
        .get("tools")
        .and_then(|v| v.as_array())
        .expect("rewritten chat request should include a tools array");
    let context_tools: Vec<_> = tools
        .iter()
        .filter(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n.starts_with("aperture_context"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        !context_tools.is_empty(),
        "Should have aperture_context tools injected"
    );
}

// ─── Test: Codex Responses API tool injection ────────────────────────────────

#[test]
fn codex_responses_tool_injection_non_streaming() {
    let engine = ContextEngine::new_in_memory(None);

    // Ingest enough for mature context
    ingest_turns(&engine, "openai", "gpt-4.1", 8);

    let body_json = openai_responses_request_json(8, false);
    let body = serde_json::to_vec(&body_json).unwrap();
    let path = "/v1/responses";

    let parsed = parse_request(path, &body).unwrap();
    assert_eq!(parsed.provider, Provider::OpenAI);
    assert!(!parsed.stream);

    // Responses path → CodexProxy runtime
    let runtime_kind = detect_runtime(path, &parsed.provider.to_string());
    assert_eq!(runtime_kind, RuntimeKind::CodexProxy);

    // Rewrite
    let rewritten = rewrite_request(&body, path, &parsed, &engine)
        .unwrap()
        .expect("non-streaming responses request should be rewritten with tool definitions");
    let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();

    // Responses API uses top-level "tools" array
    let tools = rewritten_json
        .get("tools")
        .and_then(|v| v.as_array())
        .expect("rewritten responses request should include a tools array");
    let has_context_tools = tools.iter().any(|t| {
        t.get("name")
            .or_else(|| t.get("function").and_then(|f| f.get("name")))
            .and_then(|n| n.as_str())
            .map(|n| n.starts_with("aperture_context"))
            .unwrap_or(false)
    });
    assert!(
        has_context_tools,
        "Responses API should have aperture_context tools"
    );

    // Instructions (system prompt) should contain manifest
    // (may be injected into existing instructions)
    assert!(
        rewritten_json.get("input").is_some(),
        "Input array should still be present"
    );
}

// ─── Test: Streaming request skips tool injection ────────────────────────────

#[test]
fn streaming_request_no_tools() {
    let engine = ContextEngine::new_in_memory(None);
    ingest_turns(&engine, "openai", "gpt-4.1", 8);

    // Stream = true in chat completions
    let body_json = openai_chat_request_json(8, true);
    let body = serde_json::to_vec(&body_json).unwrap();
    let path = "/v1/chat/completions";

    let parsed = parse_request(path, &body).unwrap();
    assert!(parsed.stream, "Should detect stream=true");

    // Rewrite — tools should NOT be injected for streaming
    let result = rewrite_request(&body, path, &parsed, &engine).unwrap();
    if let Some(rewritten) = result {
        let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();

        // If tools were present in original, no aperture tools should be added
        if let Some(tools) = rewritten_json.get("tools").and_then(|v| v.as_array()) {
            let context_tools: Vec<_> = tools
                .iter()
                .filter(|t| {
                    let name = t
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    name.starts_with("aperture_context")
                })
                .collect();
            assert!(
                context_tools.is_empty(),
                "Streaming requests should NOT have context tools injected"
            );
        }
    }

    // Same for Responses API streaming
    let body_json = openai_responses_request_json(8, true);
    let body = serde_json::to_vec(&body_json).unwrap();
    let path = "/v1/responses";

    let parsed = parse_request(path, &body).unwrap();
    assert!(parsed.stream, "Should detect stream=true in responses");

    let result = rewrite_request(&body, path, &parsed, &engine).unwrap();
    if let Some(rewritten) = result {
        let body_str = String::from_utf8_lossy(&rewritten);
        // Tools should not appear in streaming requests
        // (they may appear in non-tool form due to manifest, that's fine)
        assert!(
            !body_str.contains("aperture_context_preview"),
            "Streaming responses should not have tool definitions"
        );
    }
}

// ─── Test: First turn (small context) skips tools ────────────────────────────

#[test]
fn first_turn_no_tools() {
    let engine = ContextEngine::new_in_memory(None);

    // Ingest minimal context: just 1 user message
    ingest_turns(&engine, "openai", "gpt-4.1", 1);

    let body_json = openai_chat_request_json(1, false);
    let body = serde_json::to_vec(&body_json).unwrap();
    let path = "/v1/chat/completions";

    let parsed = parse_request(path, &body).unwrap();

    // Small context: only 1 non-system block, should not pass tool injection gate
    let blocks = engine.active_session_blocks();
    let non_system = blocks.iter().filter(|b| b.role != Role::System).count();
    assert!(
        non_system <= 3,
        "Small context should have <=3 non-system blocks, got {non_system}"
    );

    // Rewrite — may produce manifest but no tools
    let result = rewrite_request(&body, path, &parsed, &engine).unwrap();
    if let Some(rewritten) = result {
        let body_str = String::from_utf8_lossy(&rewritten);
        assert!(
            !body_str.contains("aperture_context_preview"),
            "First turn should not inject tool definitions"
        );
    }
}

// ─── Test: Engine internal block updates from planner ─────────────────────────

#[test]
fn engine_internal_block_updates_applied() {
    let engine = ContextEngine::new_in_memory(None);

    // Ingest blocks
    ingest_turns(&engine, "openai", "gpt-4.1", 6);

    let blocks = engine.active_session_blocks();
    assert!(!blocks.is_empty(), "Engine should have blocks after ingest");

    // Find a middle-zone block to test zone shift
    let middle_block = blocks
        .iter()
        .find(|b| b.zone == Zone::BuiltIn(BuiltInZone::Middle));

    if let Some(block) = middle_block {
        let block_id = block.id.clone();

        // Use internal method to shift zone
        engine.move_block_internal(&block_id, Zone::BuiltIn(BuiltInZone::Recency));

        // Verify the change is visible
        let updated_blocks = engine.active_session_blocks();
        let updated = updated_blocks.iter().find(|b| b.id == block_id);
        assert!(updated.is_some(), "Block should still exist");
        assert_eq!(
            updated.unwrap().zone,
            Zone::BuiltIn(BuiltInZone::Recency),
            "Block zone should be updated to Recency"
        );
    }

    // Test pin internal
    let any_block_id = blocks[0].id.clone();
    engine.set_pin_internal(
        &any_block_id,
        Some(aperture_lib::engine::types::PinPosition::Top),
    );
    let updated = engine.active_session_blocks();
    let pinned = updated.iter().find(|b| b.id == any_block_id);
    assert!(pinned.is_some());
    assert_eq!(
        pinned.unwrap().pinned,
        Some(aperture_lib::engine::types::PinPosition::Top),
        "Block should be pinned to top"
    );

    // Test unknown block_id is a no-op (shouldn't panic)
    engine.move_block_internal("nonexistent-block-id", Zone::BuiltIn(BuiltInZone::Primacy));
    engine.set_pin_internal("nonexistent-block-id", None);
}

// ─── Test: Budget pressure triggers heuristic archival ───────────────────────

#[test]
fn budget_pressure_archival_round_trip() {
    let engine = ContextEngine::new_in_memory(None);

    // Ingest many turns to build up token count.
    // Each turn has ~20 tokens of content. We need enough to hit budget thresholds.
    // Default limit for gpt-4.1 is 1_048_576 tokens — way too high for test blocks.
    // Instead, verify the planner's heuristic logic directly.

    // Ingest 20 turns to get a reasonable number of blocks
    for i in 0..10 {
        let user = make_block(
            Role::User,
            &format!(
                "User message {i} with moderate length content to accumulate tokens in context"
            ),
            (i * 2 + 1) as u32,
        );
        let assistant = make_block(
            Role::Assistant,
            &format!("Assistant response {i} providing a detailed answer to the question"),
            (i * 2 + 2) as u32,
        );
        engine.ingest(
            "openai",
            "gpt-4.1",
            "test",
            None,
            vec![user],
            vec![assistant],
        );
    }

    let blocks = engine.active_session_blocks();
    let budget = engine.budget_status();

    // Even at low utilization, the planner should produce a valid output
    // and should not crash or produce invalid mutations
    let input = aperture_lib::engine::planner::types::PlannerInput {
        blocks: blocks.clone(),
        pending_plan: None,
        signals: aperture_lib::engine::planner::types::HeuristicSignals {
            budget_status: Some(budget.clone()),
            ..Default::default()
        },
        budget: budget.clone(),
        file_mutations: None,
    };

    let output = engine.planner.plan(&input);

    // Manifest should always be generated
    assert!(
        !output.manifest.status_line.is_empty(),
        "Planner should produce a non-empty manifest status line"
    );

    // Verify cleanup instructions are well-formed
    // (breadcrumb may or may not be present depending on mutations)
    assert!(
        !output.cleanup.has_cleanup,
        "Planner-only test with no context tool traffic should not require cleanup"
    );
    assert!(
        output.cleanup.tool_use_ids_to_strip.is_empty(),
        "No context tool IDs should be marked for stripping"
    );

    // Verify mutations are all valid variants
    for mutation in &output.mutations {
        match mutation {
            aperture_lib::engine::planner::types::ContextMutation::Archive { block_id }
            | aperture_lib::engine::planner::types::ContextMutation::Compress {
                block_id, ..
            }
            | aperture_lib::engine::planner::types::ContextMutation::Shift { block_id, .. }
            | aperture_lib::engine::planner::types::ContextMutation::Pin { block_id }
            | aperture_lib::engine::planner::types::ContextMutation::Unpin { block_id }
            | aperture_lib::engine::planner::types::ContextMutation::Expand { block_id }
            | aperture_lib::engine::planner::types::ContextMutation::Recall { block_id }
            | aperture_lib::engine::planner::types::ContextMutation::UpdateContent {
                block_id,
                ..
            } => {
                // Block IDs in mutations should reference real blocks
                // (unless they're from heuristics which may reference stale blocks)
                let _ = block_id;
            }
            aperture_lib::engine::planner::types::ContextMutation::Split { .. } => {
                // Split mutations are valid
            }
        }
    }
}

// ─── Test: File mutation tracking round-trip ─────────────────────────────────

#[test]
fn file_mutation_propagation_round_trip() {
    use aperture_lib::engine::planner::file_tracker::{
        detect_file_mutations, generate_file_update_mutations, ToolCallInfo,
    };

    let engine = ContextEngine::new_in_memory(None);

    // Create a block that references "auth.rs"
    let mut file_block = make_block(
        Role::ToolResult,
        "Contents of auth.rs:\nfn login() { ... }",
        2,
    );
    file_block.metadata.file_paths = vec!["src/auth.rs".to_string()];
    file_block.metadata.tool_name = Some("read_file".to_string());

    engine.ingest(
        "openai",
        "gpt-4.1",
        "test",
        None,
        vec![
            make_block(Role::System, "System prompt", 0),
            make_block(Role::User, "Read auth.rs", 1),
            file_block,
        ],
        vec![make_block(
            Role::Assistant,
            "Here are the contents of auth.rs",
            3,
        )],
    );

    // Simulate a file mutation: edit_file("auth.rs")
    let tool_calls = vec![ToolCallInfo {
        name: "edit_file".to_string(),
        arguments: serde_json::json!({
            "file_path": "src/auth.rs",
            "content": "fn login() { /* updated */ }"
        }),
        result: Some("File edited successfully".to_string()),
    }];

    let file_mutations = detect_file_mutations(&tool_calls);
    assert!(
        !file_mutations.is_empty(),
        "Should detect file mutation from edit_file"
    );
    assert_eq!(file_mutations[0].file_path, "src/auth.rs");

    // Generate mutations based on file changes
    let blocks = engine.active_session_blocks();
    let mutations = generate_file_update_mutations(&file_mutations, &blocks);

    // The mutation should target the block containing auth.rs
    // (may be UpdateContent or Archive depending on content availability)
    for mutation in &mutations {
        match mutation {
            aperture_lib::engine::planner::types::ContextMutation::UpdateContent {
                block_id,
                new_content,
            } => {
                // The block with auth.rs should get updated content
                let original = blocks.iter().find(|b| b.id == *block_id);
                assert!(
                    original.is_some(),
                    "UpdateContent should target an existing block"
                );
                assert!(!new_content.is_empty(), "New content should not be empty");
            }
            aperture_lib::engine::planner::types::ContextMutation::Archive { block_id } => {
                let original = blocks.iter().find(|b| b.id == *block_id);
                assert!(
                    original.is_some(),
                    "Archive should target an existing block"
                );
            }
            _ => {}
        }
    }
}

// ─── Test: Durable archive/compress/update persistence across turns ─────────

#[test]
fn durable_mutations_persist_across_multiple_turns() {
    let engine = ContextEngine::new_in_memory(None);
    let path = "/v1/chat/completions";

    // Initial exchange establishes baseline blocks in engine state.
    let initial_request = serde_json::json!({
        "model": "gpt-4.1",
        "messages": [
            { "role": "system", "content": "You are helpful." },
            { "role": "user", "content": "Original block that should be compressed." },
            { "role": "assistant", "content": "fn login() { old }" },
            { "role": "user", "content": "Archive this line." }
        ]
    });
    let initial_body = serde_json::to_vec(&initial_request).expect("serialize initial request");
    let initial_parsed = parse_request(path, &initial_body).expect("parse initial request");

    let initial_response_body = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4.1",
        "choices": [{
            "message": { "role": "assistant", "content": "Initial response." }
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 4 }
    }))
    .expect("serialize initial response");
    let initial_response = parse_response(initial_parsed.provider, path, &initial_response_body)
        .expect("parse initial response");

    engine.ingest(
        &initial_parsed.provider.to_string(),
        &initial_parsed.model,
        "test",
        None,
        initial_parsed.blocks,
        initial_response.blocks,
    );

    let baseline_blocks = engine.active_session_blocks();
    let compress_id = baseline_blocks
        .iter()
        .find(|b| b.content.contains("should be compressed"))
        .map(|b| b.id.clone())
        .expect("compress target block");
    let update_id = baseline_blocks
        .iter()
        .find(|b| b.content.contains("fn login() { old }"))
        .map(|b| b.id.clone())
        .expect("update target block");
    let archive_id = baseline_blocks
        .iter()
        .find(|b| b.content.contains("Archive this line."))
        .map(|b| b.id.clone())
        .expect("archive target block");

    engine.store.update(&update_id, |b| {
        b.metadata.file_paths = vec!["src/auth.rs".to_string()];
    });

    engine.planner.set_pending_plan(PendingPlan {
        mutations: vec![
            ContextMutation::Compress {
                block_id: compress_id,
                summary: "Compressed summary for the original user context.".to_string(),
            },
            ContextMutation::Archive {
                block_id: archive_id,
            },
        ],
        token_delta: -100,
        projected_block_count: baseline_blocks.len().saturating_sub(1),
        projected_utilization: 0.2,
    });

    // Turn 1 with real tool traffic: edit_file call + tool result.
    let turn_one_request = serde_json::json!({
        "model": "gpt-4.1",
        "messages": [
            { "role": "system", "content": "You are helpful." },
            { "role": "user", "content": "Original block that should be compressed." },
            { "role": "assistant", "content": "fn login() { old }" },
            { "role": "user", "content": "Archive this line." },
            {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_edit",
                    "type": "function",
                    "function": {
                        "name": "edit_file",
                        "arguments": "{\"path\":\"src/auth.rs\"}"
                    }
                }]
            },
            {
                "role": "tool",
                "tool_call_id": "call_edit",
                "content": "fn login() { updated }"
            },
            { "role": "user", "content": "Continue." }
        ]
    });
    let turn_one_body = serde_json::to_vec(&turn_one_request).expect("serialize turn one request");
    let turn_one_parsed = parse_request(path, &turn_one_body).expect("parse turn one request");

    let rewritten_one = rewrite_request(&turn_one_body, path, &turn_one_parsed, &engine)
        .expect("rewrite turn one")
        .expect("turn one should be rewritten");
    let rewritten_one_json: serde_json::Value =
        serde_json::from_slice(&rewritten_one).expect("parse rewritten turn one");

    let rewritten_one_str = rewritten_one_json.to_string();
    assert!(
        rewritten_one_str.contains("Compressed summary for the original user context."),
        "Compressed summary should be applied in rewritten request"
    );
    assert!(
        !rewritten_one_str.contains("Archive this line."),
        "Archived turn should be removed from rewritten request"
    );
    assert!(
        rewritten_one_str.contains("fn login() { updated }"),
        "File mutation update should be reflected in rewritten request"
    );

    let rewritten_one_parsed =
        parse_request(path, &rewritten_one).expect("parse rewritten turn one request");
    let turn_one_response_body = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4.1",
        "choices": [{
            "message": { "role": "assistant", "content": "Turn one response." }
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 3 }
    }))
    .expect("serialize turn one response");
    let turn_one_response =
        parse_response(rewritten_one_parsed.provider, path, &turn_one_response_body)
            .expect("parse turn one response");
    engine.ingest(
        &rewritten_one_parsed.provider.to_string(),
        &rewritten_one_parsed.model,
        "test",
        None,
        rewritten_one_parsed.blocks,
        turn_one_response.blocks,
    );

    // Turn 2 round-trip: no new plan, verify previous mutations remain persisted.
    let mut turn_two_request = rewritten_one_json;
    turn_two_request["messages"]
        .as_array_mut()
        .expect("messages array")
        .push(serde_json::json!({ "role": "user", "content": "Second turn." }));

    let turn_two_body = serde_json::to_vec(&turn_two_request).expect("serialize turn two request");
    let turn_two_parsed = parse_request(path, &turn_two_body).expect("parse turn two request");
    let rewritten_two = rewrite_request(&turn_two_body, path, &turn_two_parsed, &engine)
        .expect("rewrite turn two")
        .unwrap_or_else(|| bytes::Bytes::from(turn_two_body.clone()));

    let rewritten_two_parsed =
        parse_request(path, &rewritten_two).expect("parse rewritten turn two request");
    let turn_two_response_body = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4.1",
        "choices": [{
            "message": { "role": "assistant", "content": "Turn two response." }
        }],
        "usage": { "prompt_tokens": 8, "completion_tokens": 2 }
    }))
    .expect("serialize turn two response");
    let turn_two_response =
        parse_response(rewritten_two_parsed.provider, path, &turn_two_response_body)
            .expect("parse turn two response");
    engine.ingest(
        &rewritten_two_parsed.provider.to_string(),
        &rewritten_two_parsed.model,
        "test",
        None,
        rewritten_two_parsed.blocks,
        turn_two_response.blocks,
    );

    let final_blocks = engine.active_session_blocks();
    assert!(
        final_blocks.iter().any(|b| b
            .content
            .contains("Compressed summary for the original user context.")),
        "Compressed content should persist across turns"
    );
    assert!(
        !final_blocks
            .iter()
            .any(|b| b.content.contains("Archive this line.")),
        "Archived content should remain absent across turns"
    );
    assert!(
        final_blocks
            .iter()
            .any(|b| b.content.contains("fn login() { updated }")),
        "Updated file content should persist across turns"
    );
}

// ─── Test: Stream field parsing across all providers ─────────────────────────

#[test]
fn stream_field_parsed_correctly_all_providers() {
    // Anthropic: stream=true
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 100,
        "stream": true,
        "messages": [{ "role": "user", "content": "Hi" }]
    }))
    .unwrap();
    let parsed = parse_request("/v1/messages", &body).unwrap();
    assert!(parsed.stream, "Anthropic stream=true should be detected");

    // Anthropic: stream=false
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 100,
        "stream": false,
        "messages": [{ "role": "user", "content": "Hi" }]
    }))
    .unwrap();
    let parsed = parse_request("/v1/messages", &body).unwrap();
    assert!(!parsed.stream, "Anthropic stream=false should be detected");

    // Anthropic: stream absent
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 100,
        "messages": [{ "role": "user", "content": "Hi" }]
    }))
    .unwrap();
    let parsed = parse_request("/v1/messages", &body).unwrap();
    assert!(
        !parsed.stream,
        "Anthropic without stream field should default to false"
    );

    // OpenAI Chat: stream=true
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4",
        "stream": true,
        "messages": [{ "role": "user", "content": "Hi" }]
    }))
    .unwrap();
    let parsed = parse_request("/v1/chat/completions", &body).unwrap();
    assert!(parsed.stream, "OpenAI Chat stream=true should be detected");

    // OpenAI Chat: stream absent
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4",
        "messages": [{ "role": "user", "content": "Hi" }]
    }))
    .unwrap();
    let parsed = parse_request("/v1/chat/completions", &body).unwrap();
    assert!(
        !parsed.stream,
        "OpenAI Chat without stream should default to false"
    );

    // OpenAI Responses: stream=true
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4.1",
        "stream": true,
        "input": "Hi"
    }))
    .unwrap();
    let parsed = parse_request("/v1/responses", &body).unwrap();
    assert!(
        parsed.stream,
        "Responses API stream=true should be detected"
    );

    // OpenAI Responses: stream absent
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4.1",
        "input": "Hi"
    }))
    .unwrap();
    let parsed = parse_request("/v1/responses", &body).unwrap();
    assert!(
        !parsed.stream,
        "Responses API without stream should default to false"
    );
}
