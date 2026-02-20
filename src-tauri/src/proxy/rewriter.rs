//! JSON-level payload rewriting for context mutations.
//!
//! Applies `RewriteDecisions` to the raw JSON request body, working at the
//! `serde_json::Value` level to avoid lossy Block->JSON round-trips.

mod payload;
mod sanitize;
mod signals;
#[cfg(test)]
mod tests;
mod trailing;

use bytes::Bytes;
use serde_json::Value;
use tracing::{debug, warn};

use self::payload::apply_decisions_to_json;
use self::sanitize::{
    sanitize_anthropic_message_structure, sanitize_anthropic_orphan_tool_results,
    sanitize_anthropic_orphan_tool_uses,
};
use self::signals::collect_traffic_signals;
use self::trailing::inject_trailing_context;
use crate::engine::planner::applicator::{apply_mutations, EngineUpdateKind, RewriteDecisions};
use crate::engine::planner::types::PlannerInput;
use crate::engine::types::Role;
use crate::engine::ContextEngine;
use crate::metacog::{self, detect_runtime, RuntimeKind};
use crate::proxy::parser::{is_messages_path, ParsedRequest};

/// Errors that can occur during payload rewriting.
#[derive(Debug, thiserror::Error)]
pub enum RewriteError {
    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("Rewrite failed: {0}")]
    Internal(String),
}

/// Rewrite a request payload with context mutations, cleanup, and trailing context.
///
/// `request_blocks` are blocks parsed from the original (pre-rewrite) request body.
/// They include content that stateless clients re-send even if previously archived.
/// This is critical for persistent archival: block IDs from here are checked against
/// `persistent_archived_ids` so re-sent content gets stripped again.
///
/// Returns `Ok(Some(rewritten_bytes))` if changes were made, `Ok(None)` if no
/// rewriting was needed.
pub fn rewrite_request(
    body: &[u8],
    path: &str,
    parsed: &ParsedRequest,
    engine: &ContextEngine,
    request_blocks: &[crate::engine::block::Block],
) -> Result<Option<Bytes>, RewriteError> {
    // Detect runtime early (needed for cleanup/sanitization even on cold start).
    let provider_str = parsed.provider.to_string();
    let session_id = engine.resolve_session(
        &provider_str,
        &parsed.model,
        "proxy",
        parsed.thread_identity.as_deref(),
    );
    let runtime_kind = detect_runtime(path, &provider_str);
    let runtime = metacog::select_runtime(runtime_kind, path);

    // Get enriched blocks from engine
    let blocks = engine.session_blocks(&session_id);
    if blocks.is_empty() {
        // H2 indicator: if there's a pending plan but blocks are empty, the plan
        // won't be consumed (cold-start path skips plan_for_session).
        if engine.planner.has_pending_plan_for_session(&session_id) {
            warn!(
                session_id = %session_id,
                "R9-DIAG rewriter: cold-start path with pending plan — plan will NOT fire (H2 candidate)"
            );
        }
        // Cold-start path: still run runtime cleanup + Anthropic orphan sanitization.
        // This prevents malformed tool_result histories from reaching upstream APIs.
        let mut json: Value = serde_json::from_slice(body)?;
        let cleanup_result = runtime.cleanup_history(&mut json);
        let mut changed =
            cleanup_result.tool_uses_stripped > 0 || cleanup_result.tool_results_stripped > 0;

        if is_messages_path(path) {
            let removed = sanitize_anthropic_orphan_tool_results(&mut json);
            if removed > 0 {
                warn!(
                    removed,
                    "Removed orphan Anthropic tool_result blocks before forwarding"
                );
                changed = true;
            }
            let removed_uses = sanitize_anthropic_orphan_tool_uses(&mut json);
            if removed_uses > 0 {
                warn!(
                    removed_uses,
                    "Removed orphan Anthropic tool_use blocks before forwarding"
                );
                changed = true;
            }
            let structural_fixes = sanitize_anthropic_message_structure(&mut json);
            if structural_fixes > 0 {
                warn!(
                    structural_fixes,
                    "Fixed Anthropic message structure (merged consecutive roles / appended user)"
                );
                changed = true;
            }
        }

        if !changed {
            return Ok(None);
        }

        let rewritten = serde_json::to_vec(&json)?;
        return Ok(Some(Bytes::from(rewritten)));
    }

    // Take pending plan and build planner input.
    // Diagnostic tracing (R9-1): log session and plan state to distinguish H1/H2.
    let pending_plan = engine.planner.take_pending_plan_for_session(&session_id);
    if let Some(ref plan) = pending_plan {
        warn!(
            session_id = %session_id,
            mutations = plan.mutations.len(),
            blocks = blocks.len(),
            "R9-DIAG rewriter: consuming pending plan for session"
        );
    }
    let budget = engine.session_budget_status(&session_id);
    let traffic_signals = collect_traffic_signals(path, body);
    let planner_signals = engine.planner.build_heuristic_signals_for_session(
        &session_id,
        &blocks,
        &budget,
        traffic_signals.current_turn_files,
    );

    let input = PlannerInput {
        blocks: blocks.clone(),
        request_block_ids: request_blocks.iter().map(|b| b.id.clone()).collect(),
        pending_plan,
        signals: planner_signals.clone(),
        budget: budget.clone(),
        file_mutations: if traffic_signals.file_mutations.is_empty() {
            None
        } else {
            Some(traffic_signals.file_mutations)
        },
    };

    // Run planner
    let plan_output = engine.planner.plan_for_session(&session_id, &input);

    // Apply mutations to get rewrite decisions.
    // Pass both engine blocks (for metadata like compressed_versions) and
    // request blocks (for turn_index of archived blocks not in engine).
    let decisions = apply_mutations(&blocks, request_blocks, &plan_output.mutations);

    // Check for threshold-triggered budget warning.
    // Only inject a warning when the alert level crosses a boundary (e.g. Normal->Warning).
    let budget_warning = engine.planner.check_alert_level_change_for_session(
        &session_id,
        &budget,
        &blocks,
        &planner_signals,
    );
    let has_breadcrumb = plan_output.cleanup.breadcrumb.is_some();

    // Check if tools should be injected (non-passive, non-streaming, mature context)
    let should_inject =
        runtime_kind != RuntimeKind::Passive && !parsed.stream && should_inject_tools(&blocks);

    // If nothing to do, return None
    if !decisions.has_payload_changes()
        && budget_warning.is_none()
        && !has_breadcrumb
        && !should_inject
    {
        // Still apply engine-side updates even when payload is unchanged
        apply_engine_updates(engine, &decisions);
        debug!("No payload rewriting needed");
        return Ok(None);
    }

    // Parse body as JSON
    let mut json: Value = serde_json::from_slice(body)?;

    // Apply turn removals and content replacements
    if decisions.has_payload_changes() {
        apply_decisions_to_json(&mut json, path, &decisions);
        debug!(
            "Applied rewrite decisions: {} turns removed, {} content replacements",
            decisions.remove_turns.len(),
            decisions.content_replacements.len()
        );
    }

    // Clean up context tool calls from history
    let cleanup_result = runtime.cleanup_history(&mut json);
    if cleanup_result.tool_uses_stripped > 0 {
        debug!(
            "Cleaned up {} context tool uses, {} results",
            cleanup_result.tool_uses_stripped, cleanup_result.tool_results_stripped
        );
    }

    // Inject budget warning and/or breadcrumb into the last user message.
    // System prompts remain untouched to preserve cache prefix stability.
    {
        let mut trailing = String::new();
        if let Some(ref warning) = budget_warning {
            trailing.push_str(warning);
        }
        if let Some(ref bc) = plan_output.cleanup.breadcrumb {
            if !trailing.is_empty() {
                trailing.push('\n');
            }
            trailing.push_str(bc);
        }
        if !trailing.is_empty() {
            inject_trailing_context(&mut json, path, &trailing);
            debug!("Injected budget warning/breadcrumb into last user message (cache-safe)");
        }
    }

    // Tool injection - gated on non-passive, non-streaming, and mature context
    if runtime_kind != RuntimeKind::Passive && !parsed.stream && should_inject_tools(&blocks) {
        runtime.inject_tools(&mut json);
        debug!("Injected context tools into request");
    }

    // Defensive Anthropic shape validation: drop orphan tool blocks that
    // no longer have a matching partner (Anthropic rejects these with 400).
    // Both directions: tool_result without tool_use, and tool_use without tool_result.
    if is_messages_path(path) {
        let removed_results = sanitize_anthropic_orphan_tool_results(&mut json);
        if removed_results > 0 {
            warn!(
                removed_results,
                "Removed orphan Anthropic tool_result blocks before forwarding"
            );
        }
        let removed_uses = sanitize_anthropic_orphan_tool_uses(&mut json);
        if removed_uses > 0 {
            warn!(
                removed_uses,
                "Removed orphan Anthropic tool_use blocks before forwarding"
            );
        }
        let structural_fixes = sanitize_anthropic_message_structure(&mut json);
        if structural_fixes > 0 {
            warn!(
                structural_fixes,
                "Fixed Anthropic message structure (merged consecutive roles / appended user)"
            );
        }
    }

    // Apply engine-side block updates (zone shifts, pins) from planner mutations
    apply_engine_updates(engine, &decisions);

    // Re-serialize
    let rewritten = serde_json::to_vec(&json)?;
    Ok(Some(Bytes::from(rewritten)))
}

/// Determine whether tools should be injected based on context maturity.
///
/// Tools are only injected when context is large enough that shifting becomes
/// relevant (more than 3 non-system blocks, i.e., at least 2 conversation turns).
pub(crate) fn should_inject_tools(blocks: &[crate::engine::block::Block]) -> bool {
    let non_system = blocks.iter().filter(|b| b.role != Role::System).count();
    non_system > 3
}

/// Apply engine-side block updates from rewrite decisions.
fn apply_engine_updates(engine: &ContextEngine, decisions: &RewriteDecisions) {
    for update in &decisions.engine_updates {
        match &update.kind {
            EngineUpdateKind::SetZone(zone) => {
                engine.move_block_internal(&update.block_id, zone.clone());
            }
            EngineUpdateKind::SetPinned(pin) => {
                engine.set_pin_internal(&update.block_id, *pin);
            }
            EngineUpdateKind::Archive => {
                engine.archive_block_internal(&update.block_id);
            }
            EngineUpdateKind::ApplyCompression { summary } => {
                engine.apply_compression_summary_internal(&update.block_id, summary);
            }
            EngineUpdateKind::RestoreOriginal => {
                engine.restore_original_internal(&update.block_id);
            }
            EngineUpdateKind::UpdateContent { new_content } => {
                engine.update_content_internal(&update.block_id, new_content);
            }
        }
    }
    if !decisions.engine_updates.is_empty() {
        debug!(
            "Applied {} engine block updates",
            decisions.engine_updates.len()
        );
    }
}
