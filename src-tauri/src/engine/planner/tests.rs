use super::*;
use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
use crate::engine::budget::AlertLevel;
use crate::engine::planner::types::PlanActions;
use crate::engine::types::{CompressionLevel, Role, Zone};
use std::collections::HashSet;

fn mock_block(id: &str, role: Role, zone: BuiltInZone, tokens: u32) -> Block {
    Block {
        id: id.to_string(),
        role,
        block_type: None,
        content: format!("Content of block {id}"),
        tokens,
        timestamp: "2026-02-13T00:00:00Z".to_string(),
        zone: Zone::BuiltIn(zone),
        pinned: None,
        compression_level: CompressionLevel::Original,
        compressed_versions: CompressionVersions {
            original: CompressionVersion {
                content: format!("Content of block {id}"),
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
fn test_planner_basic_plan_no_mutations() {
    let planner = ContextPlanner::with_default_config();
    let input = PlannerInput {
        blocks: vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)],
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan(&input);
    assert!(output.mutations.is_empty());
    assert!(!output.manifest.status_line.is_empty());
}

#[test]
fn test_planner_applies_model_plan() {
    let planner = ContextPlanner::with_default_config();
    let pending = PendingPlan {
        mutations: vec![
            ContextMutation::Archive {
                block_id: "2".into(),
            },
            ContextMutation::Pin {
                block_id: "1".into(),
            },
        ],
        token_delta: -500,
        projected_block_count: 1,
        projected_utilization: 0.45,
    };

    let input = PlannerInput {
        blocks: vec![
            mock_block("1", Role::User, BuiltInZone::Recency, 500),
            mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
        ],
        request_block_ids: HashSet::new(),
        pending_plan: Some(pending),
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan(&input);
    assert_eq!(output.mutations.len(), 2);
    assert!(matches!(
        &output.mutations[0],
        ContextMutation::Archive { block_id } if block_id == "2"
    ));
}

#[test]
fn test_committed_archives_persist_across_subsequent_turns() {
    let planner = ContextPlanner::with_default_config();
    let session_id = "session-sticky-archive";
    let blocks = vec![
        mock_block("1", Role::User, BuiltInZone::Recency, 500),
        mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
    ];
    let budget = mock_budget(50_000, 200_000);

    let first = PlannerInput {
        blocks: blocks.clone(),
        request_block_ids: HashSet::new(),
        pending_plan: Some(PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "2".into(),
            }],
            token_delta: -500,
            projected_block_count: 1,
            projected_utilization: 0.45,
        }),
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };

    let out_first = planner.plan_for_session(session_id, &first);
    assert!(out_first
        .mutations
        .iter()
        .any(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "2")));

    // Second request: the stateless client re-sends ALL content (including
    // block "2" which was archived). request_block_ids reflects the CURRENT
    // request's blocks, so "2" appears here even though the engine archived it.
    let second = PlannerInput {
        blocks: blocks.clone(),
        request_block_ids: blocks.iter().map(|b| b.id.clone()).collect(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };
    let out_second = planner.plan_for_session(session_id, &second);
    assert!(out_second
        .mutations
        .iter()
        .any(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "2")));
}

#[test]
fn test_persistent_archival_reapply_uses_request_block_ids() {
    // Scenario: block "2" was archived. On the next request, the engine no
    // longer has block "2" (it was removed by archive_block_internal), but
    // the stateless client re-sent it. request_block_ids should contain "2"
    // so the re-apply logic finds it and generates an Archive mutation.
    let planner = ContextPlanner::with_default_config();
    let session_id = "session-reapply-request-ids";

    let all_blocks = vec![
        mock_block("1", Role::User, BuiltInZone::Recency, 500),
        mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
    ];
    let budget = mock_budget(50_000, 200_000);

    // First: archive block "2"
    let first = PlannerInput {
        blocks: all_blocks.clone(),
        request_block_ids: all_blocks.iter().map(|b| b.id.clone()).collect(),
        pending_plan: Some(PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "2".into(),
            }],
            token_delta: -500,
            projected_block_count: 1,
            projected_utilization: 0.45,
        }),
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };
    planner.plan_for_session(session_id, &first);

    // Second: engine only has block "1" (block "2" was archived/removed).
    // But request_block_ids has BOTH because the client re-sent everything.
    let engine_only = vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)];
    let second = PlannerInput {
        blocks: engine_only,
        request_block_ids: all_blocks.iter().map(|b| b.id.clone()).collect(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };
    let out = planner.plan_for_session(session_id, &second);

    // Block "2" should be re-archived because it's in request_block_ids
    assert!(
        out.mutations
            .iter()
            .any(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "2")),
        "Persistent archival re-apply should find block '2' in request_block_ids"
    );
}

#[test]
fn test_persistent_archival_reapply_ignores_absent_blocks() {
    // Scenario: block "2" was archived but is NOT in request_block_ids.
    // This means the client didn't re-send it (e.g., it was truncated).
    // The re-apply logic should NOT generate an Archive mutation.
    let planner = ContextPlanner::with_default_config();
    let session_id = "session-reapply-absent";

    let all_blocks = vec![
        mock_block("1", Role::User, BuiltInZone::Recency, 500),
        mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
    ];
    let budget = mock_budget(50_000, 200_000);

    // First: archive block "2"
    let first = PlannerInput {
        blocks: all_blocks.clone(),
        request_block_ids: all_blocks.iter().map(|b| b.id.clone()).collect(),
        pending_plan: Some(PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "2".into(),
            }],
            token_delta: -500,
            projected_block_count: 1,
            projected_utilization: 0.45,
        }),
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };
    planner.plan_for_session(session_id, &first);

    // Second: block "2" is NOT in request_block_ids (not re-sent)
    let engine_only = vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)];
    let second = PlannerInput {
        blocks: engine_only.clone(),
        request_block_ids: engine_only.iter().map(|b| b.id.clone()).collect(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };
    let out = planner.plan_for_session(session_id, &second);

    assert!(
        !out.mutations
            .iter()
            .any(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "2")),
        "Should not re-archive block '2' when it's not in request_block_ids"
    );
}

#[test]
fn test_recall_clears_persistent_archive_intent() {
    let planner = ContextPlanner::with_default_config();
    let session_id = "session-recall-archive";
    let blocks = vec![
        mock_block("1", Role::User, BuiltInZone::Recency, 500),
        mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
    ];
    let budget = mock_budget(50_000, 200_000);

    let archive_plan = PlannerInput {
        blocks: blocks.clone(),
        request_block_ids: HashSet::new(),
        pending_plan: Some(PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "2".into(),
            }],
            token_delta: -500,
            projected_block_count: 1,
            projected_utilization: 0.45,
        }),
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };
    planner.plan_for_session(session_id, &archive_plan);

    let recall_plan = PlannerInput {
        blocks: blocks.clone(),
        request_block_ids: HashSet::new(),
        pending_plan: Some(PendingPlan {
            mutations: vec![ContextMutation::Recall {
                block_id: "2".into(),
            }],
            token_delta: 0,
            projected_block_count: 2,
            projected_utilization: 0.5,
        }),
        signals: Default::default(),
        file_mutations: None,
        budget: budget.clone(),
    };
    let out_recall = planner.plan_for_session(session_id, &recall_plan);
    assert!(out_recall
        .mutations
        .iter()
        .any(|m| matches!(m, ContextMutation::Recall { block_id } if block_id == "2")));

    // After recall, block "2" should NOT be re-archived even though it
    // appears in the request (request_block_ids).
    let after_recall = PlannerInput {
        blocks: blocks.clone(),
        request_block_ids: blocks.iter().map(|b| b.id.clone()).collect(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget,
    };
    let out_after_recall = planner.plan_for_session(session_id, &after_recall);
    assert!(
        !out_after_recall
            .mutations
            .iter()
            .any(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "2")),
        "archive intent should be cleared after recall"
    );
}

#[test]
fn test_planner_last_plan_wins() {
    let planner = ContextPlanner::with_default_config();

    // Set first plan
    planner.set_pending_plan(PendingPlan {
        mutations: vec![ContextMutation::Archive {
            block_id: "old".into(),
        }],
        token_delta: -100,
        projected_block_count: 1,
        projected_utilization: 0.5,
    });

    // Set second plan (replaces first)
    planner.set_pending_plan(PendingPlan {
        mutations: vec![ContextMutation::Pin {
            block_id: "new".into(),
        }],
        token_delta: 0,
        projected_block_count: 2,
        projected_utilization: 0.5,
    });

    let plan = planner.take_pending_plan().expect("should have plan");
    assert_eq!(plan.mutations.len(), 1);
    assert!(matches!(
        &plan.mutations[0],
        ContextMutation::Pin { block_id } if block_id == "new"
    ));
}

#[test]
fn test_staged_plan_commit_lifecycle() {
    let planner = ContextPlanner::with_default_config();

    planner.set_staged_plan(PendingPlan {
        mutations: vec![ContextMutation::Pin {
            block_id: "b1".into(),
        }],
        token_delta: 0,
        projected_block_count: 1,
        projected_utilization: 0.5,
    });

    assert!(planner.has_staged_plan());
    assert!(!planner.has_pending_plan());

    let committed = planner
        .commit_staged_plan()
        .expect("staged plan should commit");
    assert_eq!(committed.mutations.len(), 1);
    assert!(!planner.has_staged_plan());
    assert!(planner.has_pending_plan());
}

#[test]
fn test_staged_plan_isolation_no_cross_session_fallback() {
    let planner = ContextPlanner::with_default_config();

    planner.set_staged_plan_for_session(
        "session-a",
        PendingPlan {
            mutations: vec![ContextMutation::Pin {
                block_id: "b1".into(),
            }],
            token_delta: 0,
            projected_block_count: 1,
            projected_utilization: 0.5,
        },
    );

    assert!(planner.has_staged_plan_for_session("session-a"));
    assert!(planner.staged_plan_for_session("session-b").is_none());
    assert!(planner
        .commit_staged_plan_for_session("session-b")
        .is_none());

    // Original session must remain unchanged.
    assert!(planner.has_staged_plan_for_session("session-a"));
    assert!(!planner.has_pending_plan_for_session("session-a"));
}

#[test]
fn test_append_staged_plan_merges_without_clobbering_other_slots() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![
        mock_block("b1", Role::User, BuiltInZone::Middle, 400),
        mock_block("b2", Role::Assistant, BuiltInZone::Middle, 500),
    ];
    let budget = mock_budget(120_000, 200_000);

    planner.set_staged_plan(PendingPlan {
        mutations: vec![ContextMutation::Pin {
            block_id: "b1".into(),
        }],
        token_delta: 0,
        projected_block_count: 2,
        projected_utilization: 0.6,
    });

    let merged = planner.append_staged_plan(
        PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "b2".into(),
            }],
            token_delta: -500,
            projected_block_count: 1,
            projected_utilization: 0.58,
        },
        &blocks,
        &budget,
    );

    assert_eq!(merged.mutations.len(), 2);
    assert!(merged
        .mutations
        .iter()
        .any(|m| matches!(m, ContextMutation::Pin { block_id } if block_id == "b1")));
    assert!(merged
        .mutations
        .iter()
        .any(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "b2")));
}

#[test]
fn test_plan_suppresses_heuristics_while_staged_plan_active() {
    let planner = ContextPlanner::with_default_config();
    planner.set_staged_plan(PendingPlan {
        mutations: vec![ContextMutation::Pin {
            block_id: "m1".into(),
        }],
        token_delta: 0,
        projected_block_count: 3,
        projected_utilization: 0.95,
    });

    let blocks = vec![
        mock_block("sys", Role::System, BuiltInZone::Primacy, 200),
        mock_block("m1", Role::User, BuiltInZone::Middle, 800),
        mock_block("m2", Role::Assistant, BuiltInZone::Middle, 900),
        mock_block("recent", Role::Assistant, BuiltInZone::Recency, 200),
    ];
    let input = PlannerInput {
        blocks,
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(190_000, 200_000),
    };

    let output = planner.plan(&input);
    assert!(
        output.mutations.is_empty(),
        "Heuristics should be suppressed while staged plan is active"
    );
}

#[test]
fn test_planner_records_delta_for_next_turn() {
    let planner = ContextPlanner::with_default_config();

    // First turn: no delta
    assert!(planner.last_delta().is_none());

    // Run with mutations
    let input = PlannerInput {
        blocks: vec![
            mock_block("1", Role::User, BuiltInZone::Recency, 500),
            mock_block("2", Role::Assistant, BuiltInZone::Middle, 1000),
        ],
        request_block_ids: HashSet::new(),
        pending_plan: Some(PendingPlan {
            mutations: vec![ContextMutation::Archive {
                block_id: "2".into(),
            }],
            token_delta: -1000,
            projected_block_count: 1,
            projected_utilization: 0.25,
        }),
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    planner.plan(&input);

    // Now delta should be recorded
    let delta = planner.last_delta().expect("should have delta");
    assert_eq!(delta.archived_ids, vec!["2"]);
    assert!(delta.net_token_delta < 0);
}

#[test]
fn test_runtime_budget_ceiling_override_affects_suggestions() {
    let planner = ContextPlanner::new(PlannerConfig {
        staleness_turn_threshold: 100,
        ..PlannerConfig::default()
    });

    let blocks = vec![
        mock_block("1", Role::User, BuiltInZone::Recency, 500),
        mock_block("2", Role::Assistant, BuiltInZone::Middle, 1000),
    ];
    let budget = mock_budget(100_000, 200_000); // 50% utilization
    let signals = types::HeuristicSignals {
        current_turn: 10,
        task_boundary_detected: true,
        ..Default::default()
    };

    // With default ceiling (80%), medium threshold is 64%.
    // 50% utilization is below critical, so Tier B should be disabled.
    let baseline_suggestions = planner.generate_archival_suggestions(&blocks, &budget, &signals);
    assert!(
        baseline_suggestions.is_empty(),
        "Tier B should be gated below critical pressure"
    );

    // Runtime ceiling override to 60% moves medium threshold to 48%.
    // At 50% utilization + task boundary, Tier B opportunistic recency
    // suggestions should become eligible.
    planner.set_budget_ceiling(0.60);
    let overridden_suggestions = planner.generate_archival_suggestions(&blocks, &budget, &signals);
    assert!(
        !overridden_suggestions.is_empty(),
        "Runtime budget ceiling override should change suggestion behavior"
    );
    assert!(
        overridden_suggestions.iter().all(|s| !s.tier.is_primary()),
        "With staleness disabled, only Tier B suggestions should appear"
    );
}

#[test]
fn test_validate_plan_valid_actions() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![
        mock_block("1", Role::User, BuiltInZone::Recency, 500),
        mock_block("2", Role::Assistant, BuiltInZone::Middle, 1000),
    ];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        archive: vec!["2".into()],
        pin: vec!["1".into()],
        ..Default::default()
    };

    let plan = planner.validate_plan(&actions, &blocks, &budget).unwrap();
    assert_eq!(plan.mutations.len(), 2);
    assert!(plan.token_delta < 0); // archiving saves tokens
    assert_eq!(plan.projected_block_count, 1);
}

#[test]
fn test_replay_projection_overstates_payload_savings_when_archive_set_is_partial_per_turn() {
    let planner = ContextPlanner::with_default_config();
    let budget = mock_budget(123_000, 200_000);

    let mut blocks = vec![
        // Turn 5 (6 total): 4 targeted, 2 not targeted
        mock_block(
            "11ef5b33-2595-5356-b8ef-67181b5cc716",
            Role::ToolResult,
            BuiltInZone::Middle,
            10_227,
        ),
        mock_block(
            "8873ea9b-a332-537b-84be-49506d780fec",
            Role::ToolResult,
            BuiltInZone::Middle,
            8_023,
        ),
        mock_block(
            "c086c48f-ebb0-5945-8d6d-b48ccfd3d705",
            Role::ToolResult,
            BuiltInZone::Middle,
            6_484,
        ),
        mock_block(
            "f59da4bf-ac81-5e99-8d73-ac41c0d3954e",
            Role::ToolResult,
            BuiltInZone::Middle,
            8_281,
        ),
        mock_block("turn5-survivor-a", Role::ToolResult, BuiltInZone::Middle, 79),
        mock_block("turn5-survivor-b", Role::ToolResult, BuiltInZone::Middle, 18),
        // Turn 7 (4 total): 3 targeted, 1 not targeted
        mock_block(
            "edf5a0d8-e84d-562a-848f-451fbc5e3835",
            Role::ToolResult,
            BuiltInZone::Middle,
            13_856,
        ),
        mock_block(
            "07bdc571-0d04-5277-add0-cc25617262ed",
            Role::ToolResult,
            BuiltInZone::Middle,
            8_584,
        ),
        mock_block(
            "5589f86c-1cb3-59a0-a070-551a24722692",
            Role::ToolResult,
            BuiltInZone::Middle,
            5_187,
        ),
        mock_block("turn7-survivor-a", Role::ToolResult, BuiltInZone::Middle, 4_157),
    ];
    for block in blocks.iter_mut().take(6) {
        block.metadata.turn_index = 5;
    }
    for block in blocks.iter_mut().skip(6) {
        block.metadata.turn_index = 7;
    }

    let actions = PlanActions {
        archive: vec![
            "edf5a0d8-e84d-562a-848f-451fbc5e3835".into(),
            "11ef5b33-2595-5356-b8ef-67181b5cc716".into(),
            "07bdc571-0d04-5277-add0-cc25617262ed".into(),
            "8873ea9b-a332-537b-84be-49506d780fec".into(),
            "f59da4bf-ac81-5e99-8d73-ac41c0d3954e".into(),
            "c086c48f-ebb0-5945-8d6d-b48ccfd3d705".into(),
            "5589f86c-1cb3-59a0-a070-551a24722692".into(),
        ],
        ..Default::default()
    };

    let plan = planner.validate_plan(&actions, &blocks, &budget).unwrap();
    // Turn-aware projection: partial turns account for stub overhead (10 tokens each).
    // 7 archived blocks × 10 stub tokens = 70 tokens overhead vs full-removal estimate.
    assert_eq!(
        plan.token_delta, -60_572,
        "turn-aware projection: partial-turn archives deduct stub overhead per block"
    );
    assert_eq!(plan.projected_block_count, blocks.len() - 7);

    let rewrite = applicator::apply_mutations(&blocks, &blocks, &plan.mutations);
    assert!(
        rewrite.remove_turns.is_empty(),
        "no full-turn removal should happen when archive targets only part of each turn"
    );
    assert!(
        rewrite.has_payload_changes(),
        "partial-turn archival now produces payload changes via content stubs"
    );
    assert_eq!(
        rewrite.partial_turn_stubs.len(),
        7,
        "each archived block in a partial turn gets a content stub"
    );
    // Verify stubs target the correct turns
    let stub_turns: std::collections::HashSet<u32> =
        rewrite.partial_turn_stubs.iter().map(|s| s.turn_index).collect();
    assert!(stub_turns.contains(&5), "stubs should target turn 5");
    assert!(stub_turns.contains(&7), "stubs should target turn 7");
    assert_eq!(
        rewrite.engine_updates.len(),
        7,
        "engine archive updates still queue for each targeted block"
    );
}

#[test]
fn test_replay_projection_overstates_payload_savings_for_fresh_a24_archive_set() {
    let planner = ContextPlanner::with_default_config();
    let budget = mock_budget(145_000, 200_000);

    let mut blocks = vec![
        // Turn 1 (3 total): 2 targeted, 1 survives
        mock_block(
            "ffaada21-d5eb-53ab-9e11-d77d6b58f6d2",
            Role::User,
            BuiltInZone::Middle,
            79,
        ),
        mock_block(
            "af45ba61-62a9-5700-975a-4b6f33316558",
            Role::User,
            BuiltInZone::Middle,
            4_218,
        ),
        mock_block("turn1-survivor-a", Role::User, BuiltInZone::Middle, 39),
        // Turn 2 (3 total): 2 targeted, 1 survives
        mock_block(
            "6918718b-a986-5eeb-97a1-741d113345d5",
            Role::User,
            BuiltInZone::Middle,
            5,
        ),
        mock_block(
            "da828bf0-415c-5557-a64c-b147c9c46395",
            Role::Assistant,
            BuiltInZone::Middle,
            7,
        ),
        mock_block("turn2-survivor-a", Role::Thinking, BuiltInZone::Middle, 58),
        // Turn 4 (5 total): 3 targeted, 2 survive
        mock_block(
            "af6de63e-b18a-50bd-ba30-f1b2fe63632e",
            Role::ToolResult,
            BuiltInZone::Middle,
            26_574,
        ),
        mock_block(
            "d7d67e9a-446e-511b-b14e-5962280ca180",
            Role::ToolResult,
            BuiltInZone::Middle,
            14_571,
        ),
        mock_block(
            "8101a5ee-c82d-58a5-a74c-5093fe9f8c0c",
            Role::ToolResult,
            BuiltInZone::Middle,
            685,
        ),
        mock_block("turn4-survivor-a", Role::ToolResult, BuiltInZone::Middle, 5_516),
        mock_block("turn4-survivor-b", Role::ToolResult, BuiltInZone::Middle, 53),
    ];
    for block in blocks.iter_mut().take(3) {
        block.metadata.turn_index = 1;
    }
    for block in blocks.iter_mut().skip(3).take(3) {
        block.metadata.turn_index = 2;
    }
    for block in blocks.iter_mut().skip(6) {
        block.metadata.turn_index = 4;
    }

    let actions = PlanActions {
        archive: vec![
            "af6de63e-b18a-50bd-ba30-f1b2fe63632e".into(),
            "d7d67e9a-446e-511b-b14e-5962280ca180".into(),
            "8101a5ee-c82d-58a5-a74c-5093fe9f8c0c".into(),
            "ffaada21-d5eb-53ab-9e11-d77d6b58f6d2".into(),
            "af45ba61-62a9-5700-975a-4b6f33316558".into(),
            "6918718b-a986-5eeb-97a1-741d113345d5".into(),
            "da828bf0-415c-5557-a64c-b147c9c46395".into(),
        ],
        ..Default::default()
    };

    let plan = planner.validate_plan(&actions, &blocks, &budget).unwrap();
    // Turn-aware projection: all 3 turns are partial, 7 stubs × 10 tokens overhead.
    assert_eq!(
        plan.token_delta, -46_069,
        "turn-aware projection: partial-turn archives deduct stub overhead per block"
    );
    assert_eq!(plan.projected_block_count, blocks.len() - 7);

    let rewrite = applicator::apply_mutations(&blocks, &blocks, &plan.mutations);
    assert!(
        rewrite.remove_turns.is_empty(),
        "fresh replay archive targets still only partially cover turns"
    );
    assert!(
        rewrite.has_payload_changes(),
        "partial-turn archival now produces payload changes via content stubs"
    );
    assert_eq!(
        rewrite.partial_turn_stubs.len(),
        7,
        "each archived block gets a content stub"
    );
    // Verify stubs target the correct turns
    let stub_turns: std::collections::HashSet<u32> =
        rewrite.partial_turn_stubs.iter().map(|s| s.turn_index).collect();
    assert!(stub_turns.contains(&1), "stubs should target turn 1");
    assert!(stub_turns.contains(&2), "stubs should target turn 2");
    assert!(stub_turns.contains(&4), "stubs should target turn 4");
    assert_eq!(rewrite.engine_updates.len(), 7);
}

#[test]
fn test_validate_plan_duplicate_archives_saturate_projection() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::Assistant, BuiltInZone::Middle, 1000)];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        archive: vec!["1".into(), "1".into(), "1".into()],
        ..Default::default()
    };

    let plan = planner.validate_plan(&actions, &blocks, &budget).unwrap();
    assert_eq!(
        plan.projected_block_count, 0,
        "duplicate archive IDs must not underflow projected block count"
    );
}

#[test]
fn test_validate_plan_duplicate_archive_and_recall_projection() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::Assistant, BuiltInZone::Middle, 1000)];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        archive: vec!["1".into(), "1".into()],
        recall: vec!["1".into(), "1".into()],
        ..Default::default()
    };

    let plan = planner.validate_plan(&actions, &blocks, &budget).unwrap();
    assert_eq!(plan.projected_block_count, 1);
}

#[test]
fn test_validate_plan_invalid_block_id() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        archive: vec!["nonexistent".into()],
        ..Default::default()
    };

    let err = planner
        .validate_plan(&actions, &blocks, &budget)
        .unwrap_err();
    assert!(err[0].contains("nonexistent"));
}

#[test]
fn test_validate_plan_invalid_zone() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        shift_to: [("1".into(), "invalid_zone".into())].into(),
        ..Default::default()
    };

    let err = planner
        .validate_plan(&actions, &blocks, &budget)
        .unwrap_err();
    assert!(err[0].contains("Invalid zone"));
}

#[test]
fn test_validate_plan_rejects_thinking_block_archival() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![
        mock_block("thinking1", Role::Thinking, BuiltInZone::Middle, 2000),
        mock_block("assistant1", Role::Assistant, BuiltInZone::Middle, 500),
    ];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        archive: vec!["thinking1".into()],
        ..Default::default()
    };

    let err = planner
        .validate_plan(&actions, &blocks, &budget)
        .unwrap_err();
    assert!(
        err[0].contains("thinking block"),
        "Should reject archival of thinking blocks, got: {}",
        err[0]
    );
}

#[test]
fn test_estimate_token_delta_archive() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 1000)];
    let mutations = vec![ContextMutation::Archive {
        block_id: "1".into(),
    }];
    let delta = planner.estimate_token_delta(&mutations, &blocks);
    assert_eq!(delta, -1000);
}

#[test]
fn test_estimate_token_delta_compress() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 1000)];
    let mutations = vec![ContextMutation::Compress {
        block_id: "1".into(),
        summary: "Short summary here.".into(), // ~20 chars -> ~5 tokens estimated
    }];
    let delta = planner.estimate_token_delta(&mutations, &blocks);
    // Should be negative (saving tokens)
    assert!(delta < 0);
}

#[test]
fn test_manifest_disabled() {
    let config = PlannerConfig {
        manifest_enabled: false,
        ..Default::default()
    };
    let planner = ContextPlanner::new(config);
    let input = PlannerInput {
        blocks: vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)],
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan(&input);
    assert!(output.manifest.status_line.is_empty());
}

#[test]
fn test_parse_builtin_zone() {
    assert_eq!(parse_builtin_zone("primacy"), Some(BuiltInZone::Primacy));
    assert_eq!(parse_builtin_zone("MIDDLE"), Some(BuiltInZone::Middle));
    assert_eq!(parse_builtin_zone("Recency"), Some(BuiltInZone::Recency));
    assert_eq!(parse_builtin_zone("custom"), None);
}

#[test]
fn test_planner_generates_breadcrumb_on_mutations() {
    let planner = ContextPlanner::with_default_config();
    let pending = PendingPlan {
        mutations: vec![ContextMutation::Archive {
            block_id: "2".into(),
        }],
        token_delta: -500,
        projected_block_count: 1,
        projected_utilization: 0.25,
    };

    let input = PlannerInput {
        blocks: vec![
            mock_block("1", Role::User, BuiltInZone::Recency, 500),
            mock_block("2", Role::Assistant, BuiltInZone::Middle, 500),
        ],
        request_block_ids: HashSet::new(),
        pending_plan: Some(pending),
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan(&input);
    assert!(output.cleanup.has_cleanup);
    let breadcrumb = output
        .cleanup
        .breadcrumb
        .as_ref()
        .expect("should have breadcrumb");
    assert!(breadcrumb.contains("archived #2"));
    assert!(breadcrumb.contains("Budget: 25%"));
}

#[test]
fn test_planner_no_breadcrumb_without_mutations() {
    let planner = ContextPlanner::with_default_config();
    let input = PlannerInput {
        blocks: vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)],
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan(&input);
    assert!(!output.cleanup.has_cleanup);
    assert!(output.cleanup.breadcrumb.is_none());
}

// ── Heuristic Integration Tests ──────────────────────────

#[test]
fn test_autonomous_heuristics_disabled_at_batch_point() {
    let planner = ContextPlanner::with_default_config();

    let mut blocks = Vec::new();
    for i in 0..5 {
        blocks.push(mock_block(
            &format!("m{i}"),
            Role::Assistant,
            BuiltInZone::Middle,
            1000,
        ));
        // Set turn_index to make blocks stale
        blocks.last_mut().unwrap().metadata.turn_index = i;
    }
    blocks.push(mock_block("recent", Role::User, BuiltInZone::Recency, 500));
    blocks.last_mut().unwrap().metadata.turn_index = 9;

    // 45% utilization — above soft threshold (40%).
    // Task boundary triggers batch point but heuristics NO LONGER auto-apply.
    let input = PlannerInput {
        blocks,
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: types::HeuristicSignals {
            current_turn: 10,
            task_boundary_detected: true,
            ..Default::default()
        },
        file_mutations: None,
        budget: mock_budget(90_000, 200_000),
    };

    let output = planner.plan(&input);

    // Autonomous heuristics disabled → zero archival mutations (unless from pending_plan)
    let archive_count = output
        .mutations
        .iter()
        .filter(|m| matches!(m, ContextMutation::Archive { .. }))
        .count();
    assert_eq!(
        archive_count, 0,
        "Autonomous heuristics should be disabled — LLM controls archival via staged planning"
    );
}

#[test]
fn test_planner_skips_heuristics_without_batch_point() {
    let planner = ContextPlanner::with_default_config();

    // First, escalate to Warning so we're already AT soft pressure.
    // Then a subsequent request at the SAME level should NOT be a batch point.
    let warmup_budget = mock_budget(90_000, 200_000); // 45% → Warning
    let warmup_blocks = vec![mock_block("b1", Role::User, BuiltInZone::Middle, 1000)];
    let warmup_signals = Default::default();
    planner.check_alert_level_change(&warmup_budget, &warmup_blocks, &warmup_signals); // consume the transition

    let mut blocks = Vec::new();
    for i in 0..5 {
        blocks.push(mock_block(
            &format!("m{i}"),
            Role::Assistant,
            BuiltInZone::Middle,
            1000,
        ));
        blocks.last_mut().unwrap().metadata.turn_index = i;
    }
    blocks.push(mock_block("recent", Role::User, BuiltInZone::Recency, 500));
    blocks.last_mut().unwrap().metadata.turn_index = 9;

    // Same 45% utilization — pressure level already Warning, no change → no batch point
    let input = PlannerInput {
        blocks,
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: types::HeuristicSignals {
            current_turn: 10,
            task_boundary_detected: false,
            ..Default::default()
        },
        file_mutations: None,
        budget: mock_budget(90_000, 200_000),
    };

    let output = planner.plan(&input);

    // Heuristics should NOT run — same pressure level, no batch point
    assert!(
        output.mutations.is_empty(),
        "Heuristics should be deferred when no batch point is present"
    );
}

#[test]
fn test_planner_model_pin_overrides_heuristic_archival() {
    let planner = ContextPlanner::with_default_config();

    let mut b1 = mock_block("b1", Role::Assistant, BuiltInZone::Middle, 2000);
    b1.metadata.turn_index = 0;
    let mut b2 = mock_block("b2", Role::Assistant, BuiltInZone::Middle, 1000);
    b2.metadata.turn_index = 1;

    // Model explicitly pins b1 — heuristics should not archive it
    let pending = PendingPlan {
        mutations: vec![ContextMutation::Pin {
            block_id: "b1".into(),
        }],
        token_delta: 0,
        projected_block_count: 2,
        projected_utilization: 0.45,
    };

    let input = PlannerInput {
        blocks: vec![b1, b2],
        request_block_ids: HashSet::new(),
        pending_plan: Some(pending),
        signals: types::HeuristicSignals {
            current_turn: 10,
            ..Default::default()
        },
        file_mutations: None,
        budget: mock_budget(90_000, 200_000), // soft pressure
    };

    let output = planner.plan(&input);

    let archived_ids: Vec<&str> = output
        .mutations
        .iter()
        .filter_map(|m| match m {
            ContextMutation::Archive { block_id } => Some(block_id.as_str()),
            _ => None,
        })
        .collect();

    // b1 was pinned by model — should NOT be archived by heuristics
    assert!(
        !archived_ids.contains(&"b1"),
        "Model-pinned block should not be archived by heuristics"
    );
}

#[test]
fn test_planner_file_mutations_generate_updates() {
    use crate::engine::planner::file_tracker::{FileMutation, FileMutationKind};

    let planner = ContextPlanner::with_default_config();

    let mut b1 = mock_block("b1", Role::ToolResult, BuiltInZone::Middle, 500);
    b1.metadata.file_paths = vec!["src/auth.rs".to_string()];
    b1.metadata.turn_index = 3;

    let input = PlannerInput {
        blocks: vec![b1],
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: Some(vec![FileMutation {
            file_path: "src/auth.rs".to_string(),
            kind: FileMutationKind::Edit,
            new_content: Some("fn updated_auth() {}".to_string()),
        }]),
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan(&input);

    let update_mutations: Vec<_> = output
        .mutations
        .iter()
        .filter(|m| matches!(m, ContextMutation::UpdateContent { .. }))
        .collect();

    assert_eq!(update_mutations.len(), 1);
    assert!(matches!(
        &update_mutations[0],
        ContextMutation::UpdateContent { block_id, new_content }
            if block_id == "b1" && new_content == "fn updated_auth() {}"
    ));
}

#[test]
fn test_planner_no_heuristics_below_threshold() {
    let planner = ContextPlanner::with_default_config();

    let mut b1 = mock_block("b1", Role::Assistant, BuiltInZone::Middle, 1000);
    b1.metadata.turn_index = 0;

    let input = PlannerInput {
        blocks: vec![b1],
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals: types::HeuristicSignals {
            current_turn: 5,
            ..Default::default()
        },
        file_mutations: None,
        budget: mock_budget(20_000, 200_000), // 10% — well below thresholds
    };

    let output = planner.plan(&input);

    // No budget pressure, staleness threshold not reached (5 < 10)
    assert!(output.mutations.is_empty());
}

#[test]
fn test_build_heuristic_signals_tracks_previous_turn_files_and_boundaries() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 100)];
    let budget = mock_budget(10_000, 200_000);

    let first = planner.build_heuristic_signals(&blocks, &budget, vec!["src/auth.rs".to_string()]);
    assert!(first.task_boundary_detected);
    assert!(first.previous_turn_files.is_empty());
    assert_eq!(first.current_turn_files, vec!["src/auth.rs".to_string()]);

    let second = planner.build_heuristic_signals(
        &blocks,
        &budget,
        vec!["src/new.rs".to_string(), "src/other.rs".to_string()],
    );
    assert!(second.task_boundary_detected);
    assert_eq!(second.previous_turn_files, vec!["src/auth.rs".to_string()]);
}

#[test]
fn test_build_heuristic_signals_normalizes_current_turn_files() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("1", Role::Assistant, BuiltInZone::Middle, 100)];
    let budget = mock_budget(10_000, 200_000);

    let signals = planner.build_heuristic_signals(
        &blocks,
        &budget,
        vec![
            "src/b.rs".to_string(),
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
        ],
    );

    assert_eq!(
        signals.current_turn_files,
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
    );
    assert_eq!(signals.current_turn, blocks[0].metadata.turn_index);
}

// ── Alert Level & Batch Point Tests ─────────────────────

#[test]
fn test_check_alert_level_change_escalation() {
    // Default config: ceiling=80%, so:
    //   soft = 80% × 0.50 = 40% utilization
    //   medium = 80% × 0.80 = 64% utilization
    //   hard = 80% × 1.00 = 80% utilization
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("b1", Role::User, BuiltInZone::Middle, 1000)];
    let signals = Default::default();

    // 25% → Normal: no warning
    let budget_normal = mock_budget(50_000, 200_000);
    assert!(planner
        .check_alert_level_change(&budget_normal, &blocks, &signals)
        .is_none());

    // 45% → Warning (above soft=40%): warning emitted
    let budget_soft = mock_budget(90_000, 200_000);
    let warning = planner.check_alert_level_change(&budget_soft, &blocks, &signals);
    assert!(warning.is_some());
    assert!(warning.unwrap().contains("Consider cleaning"));

    // Still 45% → same level: no warning
    assert!(planner
        .check_alert_level_change(&budget_soft, &blocks, &signals)
        .is_none());

    // 70% → Critical (above medium=64%): warning emitted
    let budget_medium = mock_budget(140_000, 200_000);
    let warning = planner.check_alert_level_change(&budget_medium, &blocks, &signals);
    assert!(warning.is_some());
    assert!(warning.unwrap().contains("Pause and reorganize"));

    // 85% → Emergency (above hard=80%): warning emitted
    let budget_hard = mock_budget(170_000, 200_000);
    let warning = planner.check_alert_level_change(&budget_hard, &blocks, &signals);
    assert!(warning.is_some());
    assert!(warning.unwrap().contains("EMERGENCY"));
}

#[test]
fn test_check_alert_level_change_recovery_silent() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("b1", Role::User, BuiltInZone::Middle, 1000)];
    let signals = Default::default();

    // Escalate to Warning (45% > soft=40%)
    let budget_soft = mock_budget(90_000, 200_000);
    planner.check_alert_level_change(&budget_soft, &blocks, &signals);

    // Recovery: back to 25% (Normal): should be silent
    let budget_normal = mock_budget(50_000, 200_000);
    assert!(
        planner
            .check_alert_level_change(&budget_normal, &blocks, &signals)
            .is_none(),
        "Recovery should not emit a warning"
    );
}

#[test]
fn test_check_alert_level_respects_ceiling_override() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![mock_block("b1", Role::User, BuiltInZone::Middle, 1000)];
    let signals = Default::default();

    // Override ceiling to 50%: soft = 25%, medium = 40%, hard = 50%
    planner.set_budget_ceiling(0.50);

    // 30% → Warning (above soft=25%)
    let budget = mock_budget(60_000, 200_000);
    let warning = planner.check_alert_level_change(&budget, &blocks, &signals);
    assert!(
        warning.is_some(),
        "30% should trigger warning when ceiling is 50% (soft=25%)"
    );
    assert!(warning.unwrap().contains("50%")); // ceiling referenced in message
}

#[test]
fn test_is_batch_point_pending_plan() {
    let planner = ContextPlanner::with_default_config();
    let budget = mock_budget(50_000, 200_000);
    let signals = HeuristicSignals::default();

    assert!(
        planner.is_batch_point(&budget, &signals, true),
        "Pending plan should trigger batch point"
    );
    assert!(
        !planner.is_batch_point(&budget, &signals, false),
        "No signals should not trigger batch point"
    );
}

#[test]
fn test_is_batch_point_task_boundary() {
    let planner = ContextPlanner::with_default_config();
    let budget = mock_budget(50_000, 200_000);
    let signals = HeuristicSignals {
        task_boundary_detected: true,
        ..Default::default()
    };

    assert!(
        planner.is_batch_point(&budget, &signals, false),
        "Task boundary should trigger batch point"
    );
}

#[test]
fn test_is_batch_point_pressure_level_change() {
    let planner = ContextPlanner::with_default_config();
    let signals = HeuristicSignals::default();

    // 25% utilization — Normal pressure (below soft=40%)
    let budget_normal = mock_budget(50_000, 200_000);
    assert!(!planner.is_batch_point(&budget_normal, &signals, false));

    // 45% utilization — Warning pressure (above soft=40%) → batch point
    let budget_soft = mock_budget(90_000, 200_000);
    assert!(
        planner.is_batch_point(&budget_soft, &signals, false),
        "Pressure level change should trigger batch point"
    );
}

#[test]
fn test_heuristics_generate_suggestions_on_pressure() {
    let planner = ContextPlanner::with_default_config();

    let mut blocks = Vec::new();
    for i in 0..5 {
        blocks.push(mock_block(
            &format!("m{i}"),
            Role::Assistant,
            BuiltInZone::Middle,
            1000,
        ));
        blocks.last_mut().unwrap().metadata.turn_index = i;
    }
    blocks.push(mock_block("recent", Role::User, BuiltInZone::Recency, 500));
    blocks.last_mut().unwrap().metadata.turn_index = 9;

    // 45% utilization → above soft=40%, pressure level Warning
    let budget = mock_budget(90_000, 200_000);
    let signals = types::HeuristicSignals {
        current_turn: 10,
        ..Default::default()
    };

    // Heuristics should GENERATE SUGGESTIONS (not auto-apply mutations)
    let suggestions = planner.generate_archival_suggestions(&blocks, &budget, &signals);
    assert!(
        !suggestions.is_empty(),
        "Heuristics should generate archival suggestions at soft pressure"
    );

    // But planner should NOT auto-apply them
    let input = PlannerInput {
        blocks,
        request_block_ids: HashSet::new(),
        pending_plan: None,
        signals,
        file_mutations: None,
        budget,
    };
    let output = planner.plan(&input);
    let archive_count = output
        .mutations
        .iter()
        .filter(|m| matches!(m, ContextMutation::Archive { .. }))
        .count();
    assert_eq!(
        archive_count, 0,
        "Autonomous heuristics disabled — suggestions are NOT auto-applied"
    );
}

#[test]
fn test_validate_plan_accepts_hash_prefixed_ids() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![
        mock_block("abc123", Role::User, BuiltInZone::Recency, 500),
        mock_block("def456", Role::Assistant, BuiltInZone::Middle, 700),
    ];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        archive: vec!["#def456".to_string()],
        pin: vec!["#abc123".to_string()],
        ..Default::default()
    };

    let result = planner.validate_plan(&actions, &blocks, &budget);
    assert!(result.is_ok(), "validate_plan should accept #-prefixed IDs");

    let plan = result.unwrap();
    assert_eq!(plan.mutations.len(), 2);

    // Mutations should store bare IDs (without #)
    assert!(plan
        .mutations
        .iter()
        .any(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "def456")));
    assert!(plan
        .mutations
        .iter()
        .any(|m| matches!(m, ContextMutation::Pin { block_id } if block_id == "abc123")));
}

#[test]
fn test_validate_plan_mixed_hash_and_bare_ids() {
    let planner = ContextPlanner::with_default_config();
    let blocks = vec![
        mock_block("b1", Role::User, BuiltInZone::Recency, 500),
        mock_block("b2", Role::Assistant, BuiltInZone::Middle, 300),
        mock_block("b3", Role::User, BuiltInZone::Middle, 400),
    ];
    let budget = mock_budget(50_000, 200_000);

    let actions = PlanActions {
        archive: vec!["#b2".to_string(), "b3".to_string()],
        pin: vec!["b1".to_string()],
        ..Default::default()
    };

    let result = planner.validate_plan(&actions, &blocks, &budget);
    assert!(
        result.is_ok(),
        "validate_plan should accept mixed #-prefixed and bare IDs"
    );

    let plan = result.unwrap();
    assert_eq!(plan.mutations.len(), 3);
}

// ── Turn-Aware Projection Tests ─────────────────────────

#[test]
fn test_estimate_token_delta_full_turn_archive_gets_full_savings() {
    let planner = ContextPlanner::with_default_config();

    // Single block at turn 5 — archiving it means the entire turn is removed.
    let mut b1 = mock_block("b1", Role::Assistant, BuiltInZone::Middle, 5000);
    b1.metadata.turn_index = 5;
    let blocks = vec![b1];

    let mutations = vec![ContextMutation::Archive {
        block_id: "b1".into(),
    }];
    let delta = planner.estimate_token_delta(&mutations, &blocks);
    assert_eq!(delta, -5000, "Full-turn archive should get full token savings");
}

#[test]
fn test_estimate_token_delta_partial_turn_archive_deducts_stub_overhead() {
    let planner = ContextPlanner::with_default_config();

    // Two blocks at turn 3 — archiving only one is a partial-turn archive.
    let mut b1 = mock_block("b1", Role::ToolResult, BuiltInZone::Middle, 8000);
    b1.metadata.turn_index = 3;
    let mut b2 = mock_block("b2", Role::ToolResult, BuiltInZone::Middle, 200);
    b2.metadata.turn_index = 3;
    let blocks = vec![b1, b2];

    let mutations = vec![ContextMutation::Archive {
        block_id: "b1".into(),
    }];
    let delta = planner.estimate_token_delta(&mutations, &blocks);
    // Partial-turn: -8000 + 10 (stub overhead) = -7990
    assert_eq!(
        delta, -7990,
        "Partial-turn archive should deduct stub overhead from savings"
    );
}

#[test]
fn test_estimate_token_delta_mixed_full_and_partial_turns() {
    let planner = ContextPlanner::with_default_config();

    // Turn 1: 1 block, fully archived → full savings
    let mut b1 = mock_block("b1", Role::User, BuiltInZone::Middle, 500);
    b1.metadata.turn_index = 1;
    // Turn 3: 3 blocks, 2 archived → partial turn
    let mut b2 = mock_block("b2", Role::ToolResult, BuiltInZone::Middle, 10000);
    b2.metadata.turn_index = 3;
    let mut b3 = mock_block("b3", Role::ToolResult, BuiltInZone::Middle, 6000);
    b3.metadata.turn_index = 3;
    let mut b4 = mock_block("b4", Role::ToolResult, BuiltInZone::Middle, 100);
    b4.metadata.turn_index = 3;
    let blocks = vec![b1, b2, b3, b4];

    let mutations = vec![
        ContextMutation::Archive {
            block_id: "b1".into(),
        },
        ContextMutation::Archive {
            block_id: "b2".into(),
        },
        ContextMutation::Archive {
            block_id: "b3".into(),
        },
    ];
    let delta = planner.estimate_token_delta(&mutations, &blocks);
    // Turn 1: full-turn → -500
    // Turn 3: partial (b4 survives) → -(10000-10) + -(6000-10) = -15980
    // Total: -500 + -15980 = -16480
    assert_eq!(
        delta, -16480,
        "Mixed turns: full-turn gets full savings, partial-turn deducts stub overhead"
    );
}

#[test]
fn test_estimate_token_delta_archive_plus_compress_combined() {
    let planner = ContextPlanner::with_default_config();

    // Turn 1: archived (full-turn)
    let mut b1 = mock_block("b1", Role::User, BuiltInZone::Middle, 2000);
    b1.metadata.turn_index = 1;
    // Turn 2: compressed (not affected by archive logic)
    let mut b2 = mock_block("b2", Role::Assistant, BuiltInZone::Middle, 5000);
    b2.metadata.turn_index = 2;
    let blocks = vec![b1, b2];

    let mutations = vec![
        ContextMutation::Archive {
            block_id: "b1".into(),
        },
        ContextMutation::Compress {
            block_id: "b2".into(),
            summary: "Short summary text for testing.".into(), // 30 chars → ~7 tokens
        },
    ];
    let delta = planner.estimate_token_delta(&mutations, &blocks);
    // Archive: full-turn → -2000
    // Compress: -5000 + (30/4=7) = -4993
    // Total: -6993
    assert_eq!(delta, -6993);
}

// ── Option B: Persistent Archives at Commit Time (R9-1) ─

#[test]
fn test_add_persistent_archives_at_commit_time() {
    let planner = ContextPlanner::with_default_config();
    let session = "session1";

    // Stage and commit a plan with 3 archives
    let blocks = vec![
        mock_block("b1", Role::Assistant, BuiltInZone::Middle, 1000),
        mock_block("b2", Role::Assistant, BuiltInZone::Middle, 2000),
        mock_block("b3", Role::User, BuiltInZone::Recency, 500),
    ];
    let budget = mock_budget(50_000, 200_000);
    let actions = PlanActions {
        archive: vec!["b1".into(), "b2".into()],
        ..Default::default()
    };
    let validated = planner.validate_plan(&actions, &blocks, &budget).unwrap();
    planner.set_staged_plan_for_session(session, validated);
    let committed = planner.commit_staged_plan_for_session(session).unwrap();

    // Before Option B fix: persistent_archived_ids would be EMPTY here because
    // only plan_for_session() populated them. Now call add_persistent_archives.
    planner.add_persistent_archives_for_session(session, &committed.mutations);

    // Verify persistent_archived_ids are set WITHOUT needing plan_for_session().
    // Run plan_for_session with NO pending plan — persistent archives should
    // still produce Archive mutations for re-sent blocks.
    let input = PlannerInput {
        blocks: vec![mock_block("b3", Role::User, BuiltInZone::Recency, 500)],
        request_block_ids: ["b1".to_string(), "b2".to_string(), "b3".to_string()]
            .into_iter()
            .collect(),
        pending_plan: None, // Simulate the scenario where pending plan was NOT consumed
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan_for_session(session, &input);

    // Persistent archives should re-apply b1 and b2 from request_block_ids
    let archived_ids: Vec<&str> = output
        .mutations
        .iter()
        .filter_map(|m| match m {
            ContextMutation::Archive { block_id } => Some(block_id.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        archived_ids.contains(&"b1"),
        "Persistent archive for b1 should re-apply from request_block_ids"
    );
    assert!(
        archived_ids.contains(&"b2"),
        "Persistent archive for b2 should re-apply from request_block_ids"
    );
    assert!(
        !archived_ids.contains(&"b3"),
        "b3 was never archived — should not appear"
    );
}

#[test]
fn test_add_persistent_archives_recall_removes_from_persistent_set() {
    let planner = ContextPlanner::with_default_config();
    let session = "session1";

    // First commit: archive b1 and b2
    let mutations_1 = vec![
        ContextMutation::Archive {
            block_id: "b1".into(),
        },
        ContextMutation::Archive {
            block_id: "b2".into(),
        },
    ];
    planner.add_persistent_archives_for_session(session, &mutations_1);

    // Second commit: recall b1
    let mutations_2 = vec![ContextMutation::Recall {
        block_id: "b1".into(),
    }];
    planner.add_persistent_archives_for_session(session, &mutations_2);

    // Now only b2 should be in the persistent set.
    // Verify by running plan_for_session with b1 and b2 in request_block_ids.
    let input = PlannerInput {
        blocks: vec![],
        request_block_ids: ["b1".to_string(), "b2".to_string()].into_iter().collect(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan_for_session(session, &input);
    let archived_ids: Vec<&str> = output
        .mutations
        .iter()
        .filter_map(|m| match m {
            ContextMutation::Archive { block_id } => Some(block_id.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        !archived_ids.contains(&"b1"),
        "b1 was recalled — should NOT be re-archived"
    );
    assert!(
        archived_ids.contains(&"b2"),
        "b2 was never recalled — should still be persistently archived"
    );
}

#[test]
fn test_add_persistent_archives_idempotent() {
    let planner = ContextPlanner::with_default_config();
    let session = "session1";

    // Add the same archive twice — should not cause issues
    let mutations = vec![ContextMutation::Archive {
        block_id: "b1".into(),
    }];
    planner.add_persistent_archives_for_session(session, &mutations);
    planner.add_persistent_archives_for_session(session, &mutations);

    // Verify only one archive mutation is generated (not doubled)
    let input = PlannerInput {
        blocks: vec![],
        request_block_ids: ["b1".to_string()].into_iter().collect(),
        pending_plan: None,
        signals: Default::default(),
        file_mutations: None,
        budget: mock_budget(50_000, 200_000),
    };

    let output = planner.plan_for_session(session, &input);
    let archive_count = output
        .mutations
        .iter()
        .filter(|m| matches!(m, ContextMutation::Archive { block_id } if block_id == "b1"))
        .count();

    assert_eq!(
        archive_count, 1,
        "Idempotent: duplicate persistent archives should produce exactly one mutation"
    );
}
