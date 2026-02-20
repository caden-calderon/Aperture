use super::ingest::{
    is_internal_prompt, is_regressive_semantic_collapse, is_regressive_subset_capture,
};
use super::*;
use block::{BlockMetadata, CompressionVersion, CompressionVersions};
use compression::CompressionBackendKind;
use types::{BuiltInZone, CompressionLevel, PinPosition, Role, Zone};

fn make_block(id: &str, role: Role, content: &str) -> Block {
    Block {
        id: id.to_string(),
        role,
        block_type: None,
        content: content.to_string(),
        tokens: (content.len() as u32) / 4,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        zone: Zone::BuiltIn(BuiltInZone::Middle),
        pinned: None,
        compression_level: CompressionLevel::Original,
        compressed_versions: CompressionVersions {
            original: CompressionVersion {
                content: content.to_string(),
                tokens: (content.len() as u32) / 4,
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
            turn_index: 0,
            tool_name: None,
            file_paths: vec![],
        },
    }
}

fn make_block_with_turn(id: &str, role: Role, content: &str, turn_index: u32) -> Block {
    let mut block = make_block(id, role, content);
    block.metadata.turn_index = turn_index;
    block
}

#[test]
fn test_ingest_replaces_blocks_not_accumulates() {
    let engine = ContextEngine::new_in_memory(None);

    // First ingest: 2 request blocks + 1 response block
    let req1 = vec![
        make_block("r1a", Role::System, "You are a helpful assistant"),
        make_block("r1b", Role::User, "Hello"),
    ];
    let resp1 = vec![make_block("a1", Role::Assistant, "Hi there!")];

    let result1 = engine.ingest("anthropic", "claude", "proxy", None, req1, resp1, 0);
    assert_eq!(result1.block_count, 3);
    assert_eq!(engine.store.count(), 3);

    // Second ingest: full conversation history (new UUIDs) + new response
    let req2 = vec![
        make_block("r2a", Role::System, "You are a helpful assistant"),
        make_block("r2b", Role::User, "Hello"),
        make_block("r2c", Role::Assistant, "Hi there!"),
        make_block("r2d", Role::User, "How are you?"),
    ];
    let resp2 = vec![make_block("a2", Role::Assistant, "I'm doing well!")];

    let result2 = engine.ingest("anthropic", "claude", "proxy", None, req2, resp2, 0);
    assert_eq!(result2.block_count, 5);
    // Critical: store should have exactly 5 blocks (replaced, not 3+5=8)
    assert_eq!(engine.store.count(), 5);

    // Old blocks should be gone
    assert!(engine.store.get("r1a").is_none());
    assert!(engine.store.get("r1b").is_none());
    assert!(engine.store.get("a1").is_none());

    // New blocks should be present
    assert!(engine.store.get("r2a").is_some());
    assert!(engine.store.get("a2").is_some());
}

#[test]
fn test_regressive_subset_capture_detection() {
    let old = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let subset = vec!["a".to_string(), "b".to_string()];
    let shifted = vec!["a".to_string(), "d".to_string()];

    assert!(is_regressive_subset_capture(&old, &subset));
    assert!(!is_regressive_subset_capture(&old, &old));
    assert!(!is_regressive_subset_capture(&old, &shifted));
}

#[test]
fn test_regressive_semantic_collapse_detects_severe_drop_with_ephemeral_additions() {
    let old_blocks = vec![
        make_block(
            "s1",
            Role::System,
            "x-anthropic-billing-header: a\nYou are helpful",
        ),
        make_block("u1", Role::User, "task 1"),
        make_block("a1", Role::Assistant, "ack"),
        make_block("u2", Role::User, "task 2"),
        make_block("a2", Role::Assistant, "done"),
        make_block("u3", Role::User, "task 3"),
    ];

    let new_blocks = vec![
        // Same semantic content, different ID.
        make_block(
            "s2",
            Role::System,
            "x-anthropic-billing-header: b\nYou are helpful",
        ),
        // Added ephemeral tool chatter.
        make_block("t1", Role::ToolUse, "Tool: Read"),
        make_block("t2", Role::ToolResult, "Tool Result"),
    ];

    assert!(is_regressive_semantic_collapse(&old_blocks, &new_blocks));
}

#[test]
fn test_regressive_semantic_collapse_ignores_meaningful_new_turn() {
    let old_blocks = vec![
        make_block("s1", Role::System, "You are helpful"),
        make_block("u1", Role::User, "hello"),
        make_block("a1", Role::Assistant, "ack"),
        make_block("u2", Role::User, "follow-up"),
        make_block("a2", Role::Assistant, "done"),
        make_block("u3", Role::User, "next"),
    ];

    let new_blocks = vec![
        make_block("s2", Role::System, "You are helpful"),
        make_block("u4", Role::User, "new user request"),
        make_block("a4", Role::Assistant, "new response"),
    ];

    assert!(!is_regressive_semantic_collapse(&old_blocks, &new_blocks));
}

#[test]
fn test_regressive_semantic_collapse_requires_severe_shrink() {
    let old_blocks = vec![
        make_block("s1", Role::System, "You are helpful"),
        make_block("u1", Role::User, "one"),
        make_block("a1", Role::Assistant, "one"),
        make_block("u2", Role::User, "two"),
        make_block("a2", Role::Assistant, "two"),
        make_block("u3", Role::User, "three"),
    ];

    let new_blocks = vec![
        make_block("s2", Role::System, "You are helpful"),
        make_block("u2b", Role::User, "two"),
        make_block("a2b", Role::Assistant, "two"),
        make_block("u3b", Role::User, "three"),
    ];

    assert!(!is_regressive_semantic_collapse(&old_blocks, &new_blocks));
}

#[test]
fn test_ingest_skips_regressive_subset_capture() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![
            make_block("s1", Role::System, "You are helpful"),
            make_block("u1", Role::User, "hello"),
            make_block("u2", Role::User, "follow-up"),
        ],
        vec![],
        0,
    );

    let result = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![
            make_block("s1", Role::System, "You are helpful"),
            make_block("u1", Role::User, "hello"),
        ],
        vec![],
        0,
    );

    assert_eq!(result.block_count, 3, "subset capture should be ignored");
    assert!(
        engine.store.get("u2").is_some(),
        "existing block should remain after skipped ingest"
    );
    let session = engine.active_session().expect("active session");
    assert_eq!(
        session.exchange_count, 1,
        "skipped ingest must not increment exchange count"
    );
}

#[test]
fn test_ingest_allows_shrink_when_new_blocks_are_present() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![
            make_block("s1", Role::System, "You are helpful"),
            make_block("u1", Role::User, "hello"),
            make_block("u2", Role::User, "follow-up"),
        ],
        vec![],
        0,
    );

    let result = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![
            make_block("s1", Role::System, "You are helpful"),
            make_block("u1", Role::User, "hello"),
            make_block("u3", Role::User, "replacement"),
        ],
        vec![],
        0,
    );

    assert_eq!(result.block_count, 3, "replacement ingest should proceed");
    assert!(engine.store.get("u3").is_some());
    assert!(
        engine.store.get("u2").is_none(),
        "removed block should not survive a non-regressive ingest"
    );
    let session = engine.active_session().expect("active session");
    assert_eq!(session.exchange_count, 2);
}

#[test]
fn test_ingest_session_tokens_reflect_latest() {
    let engine = ContextEngine::new_in_memory(None);

    let req1 = vec![make_block("r1", Role::User, "short")];
    let resp1 = vec![make_block("a1", Role::Assistant, "short reply")];
    engine.ingest("anthropic", "claude", "proxy", None, req1, resp1, 0);

    let session1 = engine.active_session().unwrap();
    let tokens_after_first = session1.total_tokens;

    // Second ingest — different token count
    let req2 = vec![
        make_block("r2a", Role::User, "short"),
        make_block(
            "r2b",
            Role::User,
            "this is a much longer message with more tokens",
        ),
    ];
    let resp2 = vec![make_block(
        "a2",
        Role::Assistant,
        "this is also a longer response with many more tokens than before",
    )];
    engine.ingest("anthropic", "claude", "proxy", None, req2, resp2, 0);

    let session2 = engine.active_session().unwrap();
    // Tokens should reflect the latest ingest, not accumulate
    assert_ne!(
        session2.total_tokens,
        tokens_after_first + session2.total_tokens
    );
    assert_eq!(session2.exchange_count, 2);
}

#[test]
fn test_ingest_normalizes_response_turn_to_latest_request_turn() {
    let engine = ContextEngine::new_in_memory(None);

    let request_blocks = vec![
        make_block_with_turn("u0", Role::User, "turn 0", 0),
        make_block_with_turn("a0", Role::Assistant, "turn 1", 1),
        make_block_with_turn("u1", Role::User, "turn 2", 2),
        make_block_with_turn("a1", Role::Assistant, "turn 3", 3),
        make_block_with_turn("u2", Role::User, "turn 4", 4),
        make_block_with_turn("a2", Role::Assistant, "turn 5", 5),
    ];
    // Response parsers emit turn_index=0; ingest should normalize this to latest+1.
    let response_blocks = vec![make_block_with_turn(
        "latest-assistant",
        Role::Assistant,
        "newest reply",
        0,
    )];

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        request_blocks,
        response_blocks,
        0,
    );

    let latest = engine
        .block("latest-assistant")
        .expect("response block exists");
    assert_eq!(
        latest.metadata.turn_index, 6,
        "Response block should be shifted to latest request turn + 1"
    );
    assert_eq!(
        latest.zone,
        Zone::BuiltIn(BuiltInZone::Recency),
        "Newest response must stay in recency"
    );
}

#[test]
fn test_ingest_clears_stale_dependencies_when_replacing_session_blocks() {
    let engine = ContextEngine::new_in_memory(None);

    // First exchange creates a conversation dependency edge.
    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![make_block_with_turn("u1", Role::User, "hello", 0)],
        vec![make_block_with_turn("a1", Role::Assistant, "hi", 1)],
        0,
    );
    assert_eq!(engine.dependencies.edge_count(), 1);

    // Next exchange has only one block; stale edge should be removed.
    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![make_block_with_turn("u2", Role::User, "fresh", 2)],
        vec![],
        0,
    );
    assert_eq!(
        engine.dependencies.edge_count(),
        0,
        "Dependency graph should not retain edges to replaced blocks"
    );
}

#[test]
fn test_undo_restores_previous_content_after_single_edit() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "original")],
        vec![],
        0,
    );

    let decision = engine
        .update_content("u1", "edited once", "claude-sonnet", false)
        .expect("update should succeed");
    assert!(matches!(decision, PolicyDecision::Allow));
    assert_eq!(engine.block("u1").unwrap().content, "edited once");

    engine.undo_block("u1").expect("undo should succeed");
    assert_eq!(engine.block("u1").unwrap().content, "original");
}

#[test]
fn test_ensure_session_distinguishes_model_within_provider() {
    let engine = ContextEngine::new_in_memory(None);

    let first = engine.ingest(
        "openai",
        "gpt-4o",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );
    let second = engine.ingest(
        "openai",
        "gpt-5",
        "proxy",
        None,
        vec![make_block("u2", Role::User, "hi")],
        vec![],
        0,
    );

    assert_ne!(
        first.session_id, second.session_id,
        "Different models should not be merged into one session"
    );
    assert_eq!(engine.list_sessions().len(), 2);
}

#[test]
fn test_ensure_session_distinguishes_thread_identity_within_provider_model() {
    let engine = ContextEngine::new_in_memory(None);

    let first = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-a"),
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );
    let second = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-b"),
        vec![make_block("u2", Role::User, "hi")],
        vec![],
        0,
    );

    assert_ne!(
        first.session_id, second.session_id,
        "Different direct thread IDs should not merge into one session"
    );
    assert_eq!(engine.list_sessions().len(), 2);
}

#[test]
fn test_ensure_session_reuses_session_for_same_source_and_thread_identity() {
    let engine = ContextEngine::new_in_memory(None);

    let first = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-a"),
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );
    let second = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-a"),
        vec![make_block("u2", Role::User, "hi")],
        vec![],
        0,
    );

    assert_eq!(
        first.session_id, second.session_id,
        "Same source/thread identity should reuse the same session"
    );
    assert_eq!(engine.list_sessions().len(), 1);
}

#[test]
fn test_ingest_auxiliary_session_flips_active_session_until_primary_ingests_again() {
    let engine = ContextEngine::new_in_memory(None);

    let primary = engine.ingest(
        "anthropic",
        "claude-opus-4-6",
        "proxy",
        Some("main-thread"),
        vec![make_block("main-u1", Role::User, "Primary conversation input")],
        vec![],
        0,
    );
    assert_eq!(
        engine.active_session_id().as_deref(),
        Some(primary.session_id.as_str())
    );

    let auxiliary = engine.ingest(
        "anthropic",
        "claude-haiku-4-5-20251001",
        "proxy",
        Some("topic-detector-thread"),
        vec![make_block(
            "aux-u1",
            Role::User,
            "Classify topic for routing metadata",
        )],
        vec![make_block(
            "aux-a1",
            Role::Assistant,
            r#"{"isNewTopic": false, "title": null}"#,
        )],
        0,
    );
    assert_eq!(
        engine.active_session_id().as_deref(),
        Some(auxiliary.session_id.as_str()),
        "creating an auxiliary session moves active session away from primary"
    );

    let resumed_primary = engine.ingest(
        "anthropic",
        "claude-opus-4-6",
        "proxy",
        Some("main-thread"),
        vec![make_block("main-u2", Role::User, "Continue main task")],
        vec![],
        0,
    );
    assert_eq!(
        resumed_primary.session_id, primary.session_id,
        "main thread identity should reuse the original primary session"
    );
    assert_eq!(
        engine.active_session_id().as_deref(),
        Some(primary.session_id.as_str())
    );
}

#[test]
fn test_session_info_exposes_source_and_thread_identity() {
    let engine = ContextEngine::new_in_memory(None);

    let result = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-a"),
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );

    let session = engine
        .list_sessions()
        .into_iter()
        .find(|item| item.id == result.session_id)
        .expect("session should exist");
    assert_eq!(session.source, "direct_cli_bridge");
    assert_eq!(session.thread_identity.as_deref(), Some("thread-a"));
}

#[test]
fn test_update_and_remove_keep_session_totals_in_sync() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![
            make_block("u1", Role::User, "short"),
            make_block("u2", Role::User, "tiny"),
        ],
        vec![],
        0,
    );

    let before = engine.active_session().expect("session exists");
    assert_eq!(before.block_count, 2);

    engine
        .update_content(
            "u1",
            "this message is much longer than before",
            "claude-sonnet",
            false,
        )
        .expect("update should succeed");

    let after_edit = engine.active_session().expect("session exists");
    assert!(
        after_edit.total_tokens > before.total_tokens,
        "session total tokens should reflect edited content token changes"
    );

    engine
        .remove_block("u2", true)
        .expect("remove should succeed with confirmation");

    let after_remove = engine.active_session().expect("session exists");
    assert_eq!(after_remove.block_count, 1);
    assert_eq!(after_remove.total_tokens, engine.store.total_tokens());
}

#[test]
fn test_noop_update_does_not_record_version_or_action() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "same content")],
        vec![],
        0,
    );

    let action_count_before = engine.action_log.count();
    let versions_before = engine.block_versions("u1").len();
    let decision = engine
        .update_content("u1", "same content", "claude-sonnet", false)
        .expect("no-op update should succeed");

    assert!(matches!(decision, PolicyDecision::Allow));
    assert_eq!(
        engine.action_log.count(),
        action_count_before,
        "no-op edit should not add an action log entry"
    );
    assert_eq!(
        engine.block_versions("u1").len(),
        versions_before,
        "no-op edit should not add a version snapshot"
    );
}

#[test]
fn test_noop_move_pin_and_compress_do_not_log_actions() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "stable block")],
        vec![],
        0,
    );

    let zone = engine.block("u1").expect("block exists").zone;
    let action_count_before = engine.action_log.count();

    let move_decision = engine
        .move_block("u1", zone, false)
        .expect("no-op move should succeed");
    assert!(matches!(move_decision, PolicyDecision::Allow));

    let pin_decision = engine
        .pin_block("u1", None)
        .expect("no-op pin should succeed");
    assert!(matches!(pin_decision, PolicyDecision::Allow));

    let compress_decision = engine
        .compress_block("u1", CompressionLevel::Original, false)
        .expect("no-op compression should succeed");
    assert!(matches!(compress_decision, PolicyDecision::Allow));

    assert_eq!(
        engine.action_log.count(),
        action_count_before,
        "no-op mutations should not add action log entries"
    );
}

#[test]
fn test_bulk_remove_updates_session_block_ids() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![
            make_block("u1", Role::User, "one"),
            make_block("u2", Role::User, "two"),
            make_block("u3", Role::User, "three"),
        ],
        vec![],
        0,
    );

    let ids = vec!["u1".to_string(), "u3".to_string()];
    let decision = engine
        .bulk_remove(&ids, true)
        .expect("bulk remove should succeed");
    assert!(matches!(decision, PolicyDecision::Allow));

    let session_info = engine.active_session().expect("session exists");
    assert_eq!(session_info.block_count, 1);
    assert_eq!(session_info.total_tokens, engine.store.total_tokens());

    let session = engine
        .sessions
        .active()
        .expect("active session should exist");
    assert_eq!(session.block_ids, vec!["u2".to_string()]);
}

#[test]
fn test_remove_requires_confirmation_for_pinned_blocks() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "important context")],
        vec![],
        0,
    );
    engine
        .pin_block("u1", Some(PinPosition::Top))
        .expect("pin should succeed");

    let first = engine
        .remove_block("u1", false)
        .expect("policy check should return decision");
    assert!(matches!(first, PolicyDecision::RequireConfirmation { .. }));
    assert!(
        engine.block("u1").is_some(),
        "block should remain when unconfirmed"
    );

    let second = engine
        .remove_block("u1", true)
        .expect("confirmed remove should succeed");
    assert!(matches!(second, PolicyDecision::Allow));
    assert!(engine.block("u1").is_none());
}

#[test]
fn test_bulk_remove_requires_confirmation_for_large_set() {
    let engine = ContextEngine::new_in_memory(None);

    let mut req = Vec::new();
    for i in 0..6 {
        req.push(make_block(&format!("u{i}"), Role::User, "x"));
    }
    engine.ingest("anthropic", "claude-sonnet", "proxy", None, req, vec![], 0);

    let ids: Vec<String> = (0..6).map(|i| format!("u{i}")).collect();
    let first = engine
        .bulk_remove(&ids, false)
        .expect("bulk policy check should return decision");
    assert!(matches!(first, PolicyDecision::RequireConfirmation { .. }));
    assert_eq!(engine.store.count(), 6);

    let second = engine
        .bulk_remove(&ids, true)
        .expect("confirmed bulk remove should succeed");
    assert!(matches!(second, PolicyDecision::Allow));
    assert_eq!(engine.store.count(), 0);
}

// ── System-driven block mutation tests ────────────────

#[test]
fn test_move_block_internal_changes_zone() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );

    // Initially in Recency (assigned by ingest zone logic)
    let original_zone = engine.block("u1").unwrap().zone;

    engine.move_block_internal("u1", Zone::BuiltIn(BuiltInZone::Primacy));
    let updated = engine.block("u1").unwrap();
    assert_eq!(updated.zone, Zone::BuiltIn(BuiltInZone::Primacy));
    assert_ne!(updated.zone, original_zone);
}

#[test]
fn test_move_block_internal_noop_same_zone() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );

    let zone = engine.block("u1").unwrap().zone;
    // Moving to the same zone should be a no-op
    engine.move_block_internal("u1", zone.clone());
    assert_eq!(engine.block("u1").unwrap().zone, zone);
}

#[test]
fn test_move_block_internal_unknown_id_ignored() {
    let engine = ContextEngine::new_in_memory(None);
    // Should not panic
    engine.move_block_internal("nonexistent", Zone::BuiltIn(BuiltInZone::Primacy));
}

#[test]
fn test_set_pin_internal_pins_block() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );

    assert!(engine.block("u1").unwrap().pinned.is_none());
    engine.set_pin_internal("u1", Some(PinPosition::Top));
    assert_eq!(engine.block("u1").unwrap().pinned, Some(PinPosition::Top));
}

#[test]
fn test_set_pin_internal_unpins_block() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        0,
    );

    engine.set_pin_internal("u1", Some(PinPosition::Top));
    assert_eq!(engine.block("u1").unwrap().pinned, Some(PinPosition::Top));

    engine.set_pin_internal("u1", None);
    assert!(engine.block("u1").unwrap().pinned.is_none());
}

#[test]
fn test_set_pin_internal_unknown_id_ignored() {
    let engine = ContextEngine::new_in_memory(None);
    engine.set_pin_internal("nonexistent", Some(PinPosition::Top));
}

#[test]
fn test_internal_mutations_visible_in_session_blocks() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![
            make_block("u1", Role::User, "hello"),
            make_block("u2", Role::User, "world"),
        ],
        vec![],
        0,
    );

    engine.move_block_internal("u1", Zone::BuiltIn(BuiltInZone::Primacy));
    engine.set_pin_internal("u2", Some(PinPosition::Top));

    let blocks = engine.active_session_blocks();
    let u1 = blocks.iter().find(|b| b.id == "u1").unwrap();
    let u2 = blocks.iter().find(|b| b.id == "u2").unwrap();
    assert_eq!(u1.zone, Zone::BuiltIn(BuiltInZone::Primacy));
    assert_eq!(u2.pinned, Some(PinPosition::Top));
}

#[test]
fn test_archive_block_internal_removes_block_and_updates_totals() {
    let engine = ContextEngine::new_in_memory(None);
    engine.ingest(
        "openai",
        "gpt-4.1",
        "proxy",
        None,
        vec![
            make_block("u1", Role::User, "alpha"),
            make_block("u2", Role::User, "beta"),
        ],
        vec![],
        0,
    );

    let before = engine.active_session().expect("active session");
    assert_eq!(before.block_count, 2);
    engine.archive_block_internal("u1");

    assert!(engine.block("u1").is_none());
    let after = engine.active_session().expect("active session");
    assert_eq!(after.block_count, 1);
    assert_eq!(after.total_tokens, engine.store.total_tokens());
}

#[test]
fn test_apply_compression_summary_internal_updates_block_state() {
    let engine = ContextEngine::new_in_memory(None);
    engine.ingest(
        "openai",
        "gpt-4.1",
        "proxy",
        None,
        vec![make_block(
            "u1",
            Role::User,
            "long original content for compression testing",
        )],
        vec![],
        0,
    );

    engine.apply_compression_summary_internal("u1", "short summary");
    let block = engine.block("u1").expect("block exists");
    assert_eq!(block.content, "short summary");
    assert_eq!(block.compression_level, CompressionLevel::Summarized);
    assert_eq!(
        block
            .compressed_versions
            .summarized
            .as_ref()
            .map(|v| v.content.as_str()),
        Some("short summary")
    );
}

#[test]
fn test_restore_original_internal_rehydrates_original_content() {
    let engine = ContextEngine::new_in_memory(None);
    engine.ingest(
        "openai",
        "gpt-4.1",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "original content")],
        vec![],
        0,
    );

    engine.apply_compression_summary_internal("u1", "compressed");
    engine.restore_original_internal("u1");

    let block = engine.block("u1").expect("block exists");
    assert_eq!(block.content, "original content");
    assert_eq!(block.compression_level, CompressionLevel::Original);
}

#[test]
fn test_update_content_internal_resets_compression_versions() {
    let engine = ContextEngine::new_in_memory(None);
    engine.ingest(
        "openai",
        "gpt-4.1",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "old content")],
        vec![],
        0,
    );

    engine.apply_compression_summary_internal("u1", "old summary");
    engine.update_content_internal("u1", "new canonical content");

    let block = engine.block("u1").expect("block exists");
    assert_eq!(block.content, "new canonical content");
    assert_eq!(block.compression_level, CompressionLevel::Original);
    assert_eq!(
        block.compressed_versions.original.content,
        "new canonical content"
    );
    assert!(block.compressed_versions.summarized.is_none());
}

#[test]
fn test_clear_all_sessions_requires_confirmation() {
    let engine = ContextEngine::new_in_memory(None);
    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![make_block("a1", Role::Assistant, "hi")],
        0,
    );

    let decision = engine
        .clear_all_sessions(false)
        .expect("clear should return policy decision");

    assert!(matches!(
        decision,
        PolicyDecision::RequireConfirmation { .. }
    ));
    assert!(engine.store.count() > 0);
    assert!(!engine.list_sessions().is_empty());
}

#[test]
fn test_clear_all_sessions_clears_engine_state_after_confirmation() {
    let engine = ContextEngine::new_in_memory(None);
    engine.ingest(
        "anthropic",
        "claude-sonnet",
        "proxy",
        Some("thread-a"),
        vec![make_block("u1", Role::User, "hello")],
        vec![make_block("a1", Role::Assistant, "hi")],
        0,
    );
    engine.ingest(
        "openai",
        "gpt-5",
        "direct_cli_bridge",
        Some("thread-b"),
        vec![make_block("u2", Role::User, "second")],
        vec![],
        0,
    );
    engine
        .update_content("u2", "second updated", "gpt-5", false)
        .expect("update content should succeed");

    assert!(engine.store.count() > 0);
    assert!(!engine.list_sessions().is_empty());
    assert!(engine.dependencies.edge_count() > 0);
    assert!(engine.versions.tracked_block_count() > 0);
    assert!(engine.session_identity_index.len() > 0);

    let decision = engine
        .clear_all_sessions(true)
        .expect("clear with confirmation should succeed");

    assert!(matches!(decision, PolicyDecision::Allow));
    assert_eq!(engine.store.count(), 0);
    assert!(engine.list_sessions().is_empty());
    assert!(engine.sessions.active_id().is_none());
    assert_eq!(engine.dependencies.edge_count(), 0);
    assert_eq!(engine.versions.tracked_block_count(), 0);
    assert_eq!(engine.session_identity_index.len(), 0);
    assert_eq!(engine.action_log.count(), 1);
    let record = engine.recent_actions(1);
    assert_eq!(record[0].kind, ActionKind::ClearSession);
}

#[test]
fn test_compression_settings_default_available_from_engine() {
    let engine = ContextEngine::new_in_memory(None);
    let settings = engine.compression_settings();
    assert_eq!(settings.backend, CompressionBackendKind::Auto);
    assert_eq!(settings.timeout_ms, 12_000);
    assert_eq!(settings.max_tokens, 512);
}

#[test]
fn test_set_compression_settings_normalizes_values() {
    let engine = ContextEngine::new_in_memory(None);
    engine.set_compression_settings(compression::CompressionSettings {
        backend: CompressionBackendKind::OpenRouter,
        model: Some("  custom-model  ".to_string()),
        timeout_ms: 100,
        max_tokens: 40_000,
        openrouter_base_url: Some(" https://openrouter.ai/api/v1 ".to_string()),
        openrouter_api_key_env: Some("   ".to_string()),
    });

    let settings = engine.compression_settings();
    assert_eq!(settings.backend, CompressionBackendKind::OpenRouter);
    assert_eq!(settings.model.as_deref(), Some("custom-model"));
    assert_eq!(settings.timeout_ms, 500);
    assert_eq!(settings.max_tokens, 8_192);
    assert_eq!(
        settings.openrouter_base_url.as_deref(),
        Some("https://openrouter.ai/api/v1")
    );
    assert!(settings.openrouter_api_key_env.is_none());
}

// ── Internal Prompt Filter Tests ─────────────────────────

#[test]
fn test_internal_prompt_suggestion_mode_detected() {
    let block = make_block(
        "internal",
        Role::User,
        "[SUGGESTION MODE: respond only with suggestions]",
    );
    assert!(is_internal_prompt(&block));
}

#[test]
fn test_internal_prompt_normal_user_not_filtered() {
    let block = make_block("user1", Role::User, "Hello, can you help me?");
    assert!(!is_internal_prompt(&block));
}

#[test]
fn test_internal_prompt_assistant_role_not_filtered() {
    let block = make_block("asst", Role::Assistant, "[SUGGESTION MODE: something]");
    assert!(!is_internal_prompt(&block));
}

// ── ANSI Stripping in Ingest Tests ───────────────────────

#[test]
fn test_ingest_strips_ansi_codes_from_blocks() {
    let engine = ContextEngine::new_in_memory(None);

    let req = vec![make_block(
        "u1",
        Role::User,
        "\x1b[31mError:\x1b[0m file not found",
    )];
    engine.ingest("anthropic", "claude", "proxy", None, req, vec![], 0);

    let block = engine.block("u1").expect("block exists");
    assert_eq!(block.content, "Error: file not found");
    assert_eq!(
        block.compressed_versions.original.content,
        "Error: file not found"
    );
}

#[test]
fn test_ingest_filters_suggestion_mode_blocks() {
    let engine = ContextEngine::new_in_memory(None);

    let req = vec![
        make_block(
            "internal",
            Role::User,
            "[SUGGESTION MODE: respond with suggestions only]",
        ),
        make_block("real", Role::User, "Hello!"),
    ];
    let result = engine.ingest("anthropic", "claude", "proxy", None, req, vec![], 0);

    assert_eq!(result.block_count, 1);
    assert!(engine.block("internal").is_none());
    assert!(engine.block("real").is_some());
}

// ── Overhead Tokens Tests ─────────────────────────────────

#[test]
fn test_ingest_stores_overhead_tokens_in_session() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        5000,
    );

    let session = engine.sessions.active().expect("active session");
    assert_eq!(session.overhead_tokens, 5000);
}

#[test]
fn test_budget_status_includes_overhead() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        20_000,
    );

    let session = engine.sessions.active().expect("active session");
    let budget = engine.budget_status();

    // Budget used should be message tokens + overhead
    assert_eq!(
        budget.used_tokens,
        session.total_tokens + 20_000,
        "budget used_tokens should include overhead"
    );
    assert!(
        budget.utilization > (session.total_tokens as f64 / session.token_budget as f64),
        "utilization with overhead should be higher than without"
    );
}

#[test]
fn test_overhead_updates_on_subsequent_ingest() {
    let engine = ContextEngine::new_in_memory(None);

    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![],
        10_000,
    );
    assert_eq!(engine.sessions.active().unwrap().overhead_tokens, 10_000);

    // Second ingest with different overhead replaces (not accumulates)
    engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![make_block("u2", Role::User, "world")],
        vec![],
        15_000,
    );
    assert_eq!(
        engine.sessions.active().unwrap().overhead_tokens,
        15_000,
        "overhead should reflect latest ingest, not accumulate"
    );
}

// ── IngestResult.applied Tests ──────────────────────────────

#[test]
fn test_ingest_result_applied_false_for_regressive_captures() {
    let engine = ContextEngine::new_in_memory(None);

    let first = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![
            make_block("s1", Role::System, "You are helpful"),
            make_block("u1", Role::User, "hello"),
            make_block("u2", Role::User, "follow-up"),
        ],
        vec![],
        0,
    );
    assert!(first.applied, "first ingest should apply");

    // Pure subset — regressive guard should reject
    let second = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![
            make_block("s1", Role::System, "You are helpful"),
            make_block("u1", Role::User, "hello"),
        ],
        vec![],
        0,
    );
    assert!(!second.applied, "regressive subset ingest should not apply");
}

#[test]
fn test_ingest_result_applied_true_for_normal_ingests() {
    let engine = ContextEngine::new_in_memory(None);

    let first = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hello")],
        vec![make_block("a1", Role::Assistant, "hi")],
        0,
    );
    assert!(first.applied);

    // Second ingest with new content — should apply
    let second = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![
            make_block("u1", Role::User, "hello"),
            make_block("a1", Role::Assistant, "hi"),
            make_block("u2", Role::User, "follow-up"),
        ],
        vec![make_block("a2", Role::Assistant, "response")],
        0,
    );
    assert!(second.applied, "normal ingest should apply");
}

// ── Context tool block filtering ─────────────────────────

#[test]
fn test_ingest_filters_context_tool_blocks() {
    let engine = ContextEngine::new_in_memory(None);

    let mut tu_block = make_block("ctx_tu", Role::ToolUse, "context tool use");
    tu_block.metadata.tool_name = Some("aperture_context_preview".to_string());

    let mut tr_block = make_block("ctx_tr", Role::ToolResult, "preview result");
    tr_block.metadata.tool_name = Some("aperture_context_preview".to_string());

    let mut mcp_tu = make_block("mcp_tu", Role::ToolUse, "mcp context tool use");
    mcp_tu.metadata.tool_name =
        Some("mcp__aperture__aperture_context_plan".to_string());

    let real_block = make_block("real", Role::User, "Hello!");

    let result = engine.ingest(
        "anthropic",
        "claude",
        "proxy",
        None,
        vec![tu_block, tr_block, mcp_tu, real_block],
        vec![],
        0,
    );

    assert_eq!(
        result.block_count, 1,
        "Only non-context-tool blocks should be stored"
    );
    assert!(engine.block("ctx_tu").is_none());
    assert!(engine.block("ctx_tr").is_none());
    assert!(engine.block("mcp_tu").is_none());
    assert!(engine.block("real").is_some());
}

// ── Session flip guard tests ─────────────────────────────

#[test]
fn test_auxiliary_model_session_does_not_flip_active() {
    let engine = ContextEngine::new(None);

    // Create main Opus session with substantial content
    let opus_id = engine.resolve_session("anthropic", "claude-opus-4-6", "proxy", None);
    engine.ingest(
        "anthropic",
        "claude-opus-4-6",
        "proxy",
        None,
        vec![
            make_block("sys", Role::System, "You are helpful."),
            make_block("u1", Role::User, &"x".repeat(8000)),
            make_block("a1", Role::Assistant, &"y".repeat(4000)),
        ],
        vec![],
        0,
    );
    assert_eq!(engine.active_session_id(), Some(opus_id.clone()));

    // Now Haiku classifier traffic arrives — different model
    let haiku_id =
        engine.resolve_session("anthropic", "claude-haiku-4-5-20251001", "proxy", None);

    // Haiku should get its own session but NOT become active
    assert_ne!(haiku_id, opus_id, "Haiku should get a separate session");
    assert_eq!(
        engine.active_session_id(),
        Some(opus_id.clone()),
        "Opus session should remain active after Haiku session creation"
    );

    // Subsequent Haiku requests (reusing existing session) should also not flip
    let haiku_id2 =
        engine.resolve_session("anthropic", "claude-haiku-4-5-20251001", "proxy", None);
    assert_eq!(haiku_id2, haiku_id, "Should reuse existing Haiku session");
    assert_eq!(
        engine.active_session_id(),
        Some(opus_id),
        "Opus session should remain active after Haiku session reuse"
    );
}

#[test]
fn test_same_model_session_reuse_does_flip_active() {
    let engine = ContextEngine::new(None);

    // Create first Opus session
    let opus_id = engine.resolve_session("anthropic", "claude-opus-4-6", "proxy", Some("thread-a"));
    engine.ingest(
        "anthropic",
        "claude-opus-4-6",
        "proxy",
        Some("thread-a"),
        vec![
            make_block("u1", Role::User, &"x".repeat(8000)),
        ],
        vec![],
        0,
    );
    assert_eq!(engine.active_session_id(), Some(opus_id.clone()));

    // Second Opus session (different thread) should become active
    let opus_id2 = engine.resolve_session("anthropic", "claude-opus-4-6", "proxy", Some("thread-b"));
    assert_ne!(opus_id, opus_id2);
    assert_eq!(
        engine.active_session_id(),
        Some(opus_id2),
        "Same-model session should be allowed to become active"
    );
}

#[test]
fn test_small_active_session_allows_model_flip() {
    let engine = ContextEngine::new(None);

    // Create main Opus session but with SMALL content (< 1000 tokens)
    let opus_id = engine.resolve_session("anthropic", "claude-opus-4-6", "proxy", None);
    engine.ingest(
        "anthropic",
        "claude-opus-4-6",
        "proxy",
        None,
        vec![make_block("u1", Role::User, "hi")],
        vec![],
        0,
    );
    assert_eq!(engine.active_session_id(), Some(opus_id.clone()));

    // Haiku session should become active since Opus session is small
    let haiku_id =
        engine.resolve_session("anthropic", "claude-haiku-4-5-20251001", "proxy", None);
    assert_eq!(
        engine.active_session_id(),
        Some(haiku_id),
        "Different model should become active when current active is small"
    );
}
