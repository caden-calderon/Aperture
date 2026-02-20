use super::plan::normalize_plan_arguments;
use super::*;
use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
use crate::engine::budget::AlertLevel;
use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

fn make_block(id: &str, role: Role, zone: BuiltInZone, tokens: u32, content: &str) -> Block {
    Block {
        id: id.to_string(),
        role,
        block_type: None,
        content: content.to_string(),
        tokens,
        timestamp: "2026-02-13T00:00:00Z".to_string(),
        zone: Zone::BuiltIn(zone),
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
    }
}

fn mock_budget(used: u32, limit: u32) -> BudgetStatus {
    let utilization = if limit > 0 {
        used as f64 / limit as f64
    } else {
        0.0
    };
    BudgetStatus {
        used_tokens: used,
        limit_tokens: limit,
        utilization,
        alert_level: if utilization >= 0.95 {
            AlertLevel::Emergency
        } else if utilization >= 0.90 {
            AlertLevel::Critical
        } else if utilization >= 0.80 {
            AlertLevel::Warning
        } else {
            AlertLevel::Normal
        },
        remaining_tokens: limit.saturating_sub(used),
    }
}

#[test]
fn test_context_preview_basic() {
    let blocks = vec![
        make_block("b1", Role::System, BuiltInZone::Primacy, 200, "system"),
        make_block("b2", Role::User, BuiltInZone::Recency, 120, "user content"),
    ];
    let budget = mock_budget(1_000, 10_000);
    let planner = ContextPlanner::with_default_config();

    let output = context_preview(&blocks, &budget, &planner, DEFAULT_TOOL_SESSION_ID);
    assert!(!output.is_error);
    assert!(output.content.contains("Context Preview"));
}

#[test]
fn test_context_read_found() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Recency,
        80,
        "hello",
    )];
    let output = context_read("b1", &blocks);
    assert!(!output.is_error);
    assert!(output.content.contains("Block #b1"));
}

#[test]
fn test_context_read_not_found() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Recency,
        80,
        "hello",
    )];
    let output = context_read("missing", &blocks);
    assert!(output.is_error);
}

#[test]
fn test_context_search_matches() {
    let mut block = make_block("b1", Role::User, BuiltInZone::Recency, 80, "hello world");
    block.metadata.file_paths = vec!["src/main.rs".to_string()];
    let output = context_search("hello", "active", &[block]);
    assert!(!output.is_error);
    assert!(output.content.contains("matches"));
}

#[test]
fn test_context_search_no_matches() {
    let block = make_block("b1", Role::User, BuiltInZone::Recency, 80, "hello world");
    let output = context_search("goodbye", "active", &[block]);
    assert!(!output.is_error);
    assert!(output.content.contains("No matches"));
}

#[test]
fn test_context_plan_valid_stage() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Middle,
        80,
        "hello",
    )];
    let budget = mock_budget(5_000, 20_000);
    let planner = ContextPlanner::with_default_config();

    let args = serde_json::json!({"archive": ["b1"]});
    let output = context_plan(&args, &blocks, &budget, &planner, DEFAULT_TOOL_SESSION_ID);
    assert!(!output.is_error);
    assert!(output.content.contains("Plan staged") || output.content.contains("mutations"));
}

#[test]
fn test_context_plan_invalid_block() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Middle,
        80,
        "hello",
    )];
    let budget = mock_budget(5_000, 20_000);
    let planner = ContextPlanner::with_default_config();

    let args = serde_json::json!({"archive": ["missing"]});
    let output = context_plan(&args, &blocks, &budget, &planner, DEFAULT_TOOL_SESSION_ID);
    assert!(output.is_error);
}

#[test]
fn test_normalize_plan_arguments_handles_stringified_empty_collections() {
    let normalized = normalize_plan_arguments(&serde_json::json!({
        "archive": "[]",
        "compress": "[]",
        "shift_to": "{}"
    }));

    assert_eq!(normalized["archive"], serde_json::json!([]));
    assert_eq!(normalized["compress"], serde_json::json!({}));
    assert_eq!(normalized["shift_to"], serde_json::json!({}));
}

#[test]
fn test_normalize_plan_arguments_strips_hash_prefix_from_ids() {
    let normalized = normalize_plan_arguments(&serde_json::json!({
        "archive": ["#b1", "b2"],
        "compress": {"#b1": "summary"}
    }));

    assert_eq!(normalized["archive"], serde_json::json!(["b1", "b2"]));
    assert_eq!(normalized["compress"], serde_json::json!({"b1": "summary"}));
}

#[test]
fn test_dispatch_tool_context_plan_normalizes_before_validation() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Middle,
        80,
        "hello",
    )];
    let budget = mock_budget(5_000, 20_000);
    let planner = ContextPlanner::with_default_config();

    let args = serde_json::json!({
        "archive": "[\"#b1\"]",
        "control": {"op": "stage"}
    });
    let output = dispatch_tool(
        "aperture_context_plan",
        &args,
        &blocks,
        &budget,
        &planner,
    );
    assert!(
        !output.is_error,
        "dispatch_tool should normalize plan args before validation"
    );
}

#[test]
fn test_dispatch_tool_context_plan_normalized_unknown_id_still_errors() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Middle,
        80,
        "hello",
    )];
    let budget = mock_budget(5_000, 20_000);
    let planner = ContextPlanner::with_default_config();

    let args = serde_json::json!({
        "archive": "[\"#missing\"]",
        "control": {"op": "stage"}
    });
    let output = dispatch_tool(
        "aperture_context_plan",
        &args,
        &blocks,
        &budget,
        &planner,
    );
    assert!(output.is_error);
    assert!(output.content.contains("unknown block IDs"));
}

#[test]
fn test_context_search_multi_word_query_matches_individual_terms() {
    // Multi-word query should match blocks containing individual terms
    let mut block1 = make_block(
        "b1",
        Role::Assistant,
        BuiltInZone::Middle,
        200,
        "The heuristics module handles archival decisions based on staleness.",
    );
    block1.metadata.file_paths = vec!["src/engine/planner/heuristics.rs".to_string()];

    let mut block2 = make_block(
        "b2",
        Role::ToolResult,
        BuiltInZone::Middle,
        300,
        "The applicator transforms mutations into rewrite decisions.",
    );
    block2.metadata.file_paths = vec!["src/engine/planner/applicator.rs".to_string()];

    let block3 = make_block(
        "b3",
        Role::User,
        BuiltInZone::Middle,
        50,
        "Please help me with unrelated task.",
    );

    let blocks = vec![block1, block2, block3];

    // Multi-word query — none of these blocks contain the full phrase
    let output = context_search("heuristics applicator storage", "active", &blocks);
    assert!(!output.is_error);
    // Should find matches for individual terms
    assert!(
        output.content.contains("matches"),
        "Multi-word query should match blocks with individual terms: {}",
        output.content
    );
    // b1 has "heuristics", b2 has "applicator" — both should match
    assert!(
        output.content.contains("b1") || output.content.contains("b2"),
        "Should find blocks matching individual search terms: {}",
        output.content
    );
}

#[test]
fn test_context_status() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Middle,
        80,
        "hello",
    )];
    let budget = mock_budget(5_000, 20_000);
    let planner = ContextPlanner::with_default_config();

    let output = context_status(&blocks, &budget, &planner, DEFAULT_TOOL_SESSION_ID);
    assert!(!output.is_error);
    assert!(!output.content.is_empty());
}

#[test]
fn test_context_plan_unknown_params_returns_helpful_error() {
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Middle,
        80,
        "hello",
    )];
    let budget = mock_budget(5_000, 20_000);
    let planner = ContextPlanner::with_default_config();

    // Haiku-style query that sends unknown parameters
    let args = serde_json::json!({"query": "what blocks are available?"});
    let output = context_plan(&args, &blocks, &budget, &planner, DEFAULT_TOOL_SESSION_ID);
    assert!(output.is_error, "Should return error for unknown params");
    assert!(
        output.content.contains("Unknown parameter"),
        "Error should mention unknown params: {}",
        output.content
    );
    assert!(
        output.content.contains("archive") && output.content.contains("pin"),
        "Error should list expected params: {}",
        output.content
    );
}

#[test]
fn test_dispatch_unknown_tool() {
    let planner = ContextPlanner::with_default_config();
    let out = dispatch_tool(
        "unknown",
        &serde_json::json!({}),
        &[],
        &mock_budget(0, 10_000),
        &planner,
    );
    assert!(out.is_error);
}

#[test]
fn test_context_read_truncates_large_output() {
    let big = "x".repeat(10_000);
    let blocks = vec![make_block(
        "b1",
        Role::User,
        BuiltInZone::Middle,
        2_500,
        &big,
    )];
    let out = context_read_with_limits("b1", &blocks, ToolOutputLimits::compact());
    assert!(!out.is_error);
    assert!(out.content.contains("Output truncated"));
}

#[test]
fn test_context_preview_omits_extra_blocks_when_limited() {
    let blocks: Vec<Block> = (0..20)
        .map(|i| {
            make_block(
                &format!("b{i}"),
                Role::User,
                BuiltInZone::Middle,
                50,
                "content",
            )
        })
        .collect();
    let planner = ContextPlanner::with_default_config();
    let out = context_preview_with_limits(
        &blocks,
        &mock_budget(5_000, 20_000),
        &planner,
        ToolOutputLimits::compact(),
        DEFAULT_TOOL_SESSION_ID,
    );
    assert!(out.content.contains("additional blocks omitted"));
}
