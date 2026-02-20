use aperture_lib::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
use aperture_lib::engine::budget::budget_status;
use aperture_lib::engine::planner::types::{ContextMutation, HeuristicSignals, PendingPlan};
use aperture_lib::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};
use aperture_lib::engine::ContextEngine;

fn make_block(id: &str, role: Role, content: &str) -> Block {
    Block {
        id: id.to_string(),
        role,
        block_type: None,
        content: content.to_string(),
        tokens: (content.len() as u32) / 4,
        timestamp: "2026-02-09T00:00:00Z".to_string(),
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
            provider: "openai".to_string(),
            turn_index: 0,
            tool_name: None,
            file_paths: vec![],
        },
    }
}

#[test]
fn isolates_sessions_by_source_thread_identity_for_same_provider_model() {
    let engine = ContextEngine::new_in_memory(None);

    let first = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-alpha"),
        vec![make_block("u1", Role::User, "alpha")],
        vec![],
        0,
    );
    let second = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-beta"),
        vec![make_block("u2", Role::User, "beta")],
        vec![],
        0,
    );

    assert_ne!(first.session_id, second.session_id);
    assert_eq!(engine.list_sessions().len(), 2);
}

#[test]
fn reuses_session_for_same_source_thread_identity() {
    let engine = ContextEngine::new_in_memory(None);

    let first = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-alpha"),
        vec![make_block("u1", Role::User, "alpha")],
        vec![],
        0,
    );
    let second = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-alpha"),
        vec![make_block("u2", Role::User, "beta")],
        vec![],
        0,
    );

    assert_eq!(first.session_id, second.session_id);
    assert_eq!(engine.list_sessions().len(), 1);
}

#[test]
fn planner_pending_plan_is_isolated_per_session() {
    let engine = ContextEngine::new_in_memory(None);

    let alpha = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-alpha"),
        vec![make_block("alpha-u1", Role::User, "alpha")],
        vec![],
        0,
    );
    let beta = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-beta"),
        vec![make_block("beta-u1", Role::User, "beta")],
        vec![],
        0,
    );

    engine.planner.set_pending_plan_for_session(
        &alpha.session_id,
        PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "alpha-u1".to_string(),
            }],
            token_delta: -10,
            projected_block_count: 0,
            projected_utilization: 0.1,
        },
    );

    assert!(
        !engine
            .planner
            .has_pending_plan_for_session(&beta.session_id),
        "beta session should not see alpha pending plan"
    );
    assert!(
        engine
            .planner
            .take_pending_plan_for_session(&alpha.session_id)
            .is_some(),
        "alpha session should keep its pending plan"
    );
}

#[test]
fn planner_alert_levels_are_isolated_per_session() {
    let engine = ContextEngine::new_in_memory(None);

    let alpha = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-alpha"),
        vec![make_block("alpha-u1", Role::User, "alpha")],
        vec![],
        0,
    );
    let beta = engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some("thread-beta"),
        vec![make_block("beta-u1", Role::User, "beta")],
        vec![],
        0,
    );

    let budget = budget_status(90_000, 100_000, &Default::default());
    let blocks = vec![make_block("stale-1", Role::Assistant, "stale block")];
    let signals = HeuristicSignals {
        current_turn: 50,
        ..Default::default()
    };

    let alpha_warning = engine.planner.check_alert_level_change_for_session(
        &alpha.session_id,
        &budget,
        &blocks,
        &signals,
    );
    assert!(alpha_warning.is_some());

    let beta_warning = engine.planner.check_alert_level_change_for_session(
        &beta.session_id,
        &budget,
        &blocks,
        &signals,
    );
    assert!(
        beta_warning.is_some(),
        "beta should still emit first escalation even after alpha crossed threshold"
    );
}
