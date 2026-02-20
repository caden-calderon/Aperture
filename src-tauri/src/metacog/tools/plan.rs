use serde::Deserialize;
use serde_json::Value;

use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;
use crate::engine::planner::types::PlanActions;
use crate::engine::planner::ContextPlanner;

use super::ToolOutput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanControlOp {
    Stage,
    Append,
    Preview,
    Commit,
    Discard,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PlanControl {
    #[serde(default)]
    op: Option<String>,
}

fn has_action_payload(actions: &PlanActions) -> bool {
    !actions.expand.is_empty()
        || !actions.archive.is_empty()
        || !actions.recall.is_empty()
        || !actions.pin.is_empty()
        || !actions.unpin.is_empty()
        || !actions.shift_to.is_empty()
        || !actions.compress.is_empty()
        || !actions.split.is_empty()
}

fn parse_plan_control_op(arguments: &Value, has_actions: bool) -> Result<PlanControlOp, String> {
    let control = match arguments.get("control") {
        Some(raw) => serde_json::from_value::<PlanControl>(raw.clone())
            .map_err(|e| format!("Invalid plan control: {e}"))?,
        None => PlanControl::default(),
    };

    let op = control
        .op
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(if has_actions { "stage" } else { "preview" });

    match op {
        "stage" => Ok(PlanControlOp::Stage),
        "append" => Ok(PlanControlOp::Append),
        "preview" => Ok(PlanControlOp::Preview),
        "commit" => Ok(PlanControlOp::Commit),
        "discard" => Ok(PlanControlOp::Discard),
        other => Err(format!(
            "Invalid control.op '{other}'. Expected one of: stage, append, preview, commit, discard"
        )),
    }
}

fn format_plan_preview(plan: &crate::engine::planner::types::PendingPlan, header: &str) -> String {
    let mut output = format!("{header} — {} mutations:\n", plan.mutations.len());

    for mutation in &plan.mutations {
        output.push_str(&format!("  • {}\n", super::describe_mutation(mutation)));
    }

    let delta_str = if plan.token_delta > 0 {
        format!("+{}", super::format_tokens(plan.token_delta as u32))
    } else if plan.token_delta < 0 {
        format!("-{}", super::format_tokens((-plan.token_delta) as u32))
    } else {
        "no change".into()
    };

    output.push_str(&format!(
        "\nProjected impact: {delta_str} tokens, {} blocks, {:.0}% budget\n",
        plan.projected_block_count,
        plan.projected_utilization * 100.0
    ));
    output.push_str(
        "Strategic mode active: call context_plan({\"control\":{\"op\":\"append\"}, ...}) to refine, then context_plan({\"control\":{\"op\":\"commit\"}}) to apply between turns.",
    );
    output
}

/// Normalize `aperture_context_plan` arguments before deserializing into [`PlanActions`].
///
/// LLMs sometimes send `"[]"` (a JSON string) instead of `[]` (a JSON array) for
/// list fields, or `"[]"` instead of `{}` for map fields. This pre-processing step
/// converts those mis-typed values to the correct empty collection so deserialization
/// succeeds rather than returning an unhelpful type-error to the LLM.
pub(super) fn normalize_plan_arguments(args: &Value) -> Value {
    let Some(obj) = args.as_object() else {
        return args.clone();
    };
    let mut normalized = obj.clone();

    // Array fields — normalize known placeholder strings to actual arrays,
    // and strip leading `#` from block ID strings.
    for field in &["archive", "expand", "recall", "pin", "unpin"] {
        if let Some(v) = normalized.get(*field) {
            if let Some(raw) = v.as_str() {
                let trimmed = raw.trim();
                if trimmed == "[]" {
                    normalized.insert(field.to_string(), Value::Array(vec![]));
                } else if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    if parsed.is_array() {
                        normalized.insert(field.to_string(), parsed);
                    }
                }
            }
        }

        // Strip `#` prefix from each string element in arrays
        if let Some(Value::Array(arr)) = normalized.get(*field) {
            let cleaned: Vec<Value> = arr
                .iter()
                .map(|v| match v.as_str() {
                    Some(s) => Value::String(s.strip_prefix('#').unwrap_or(s).to_string()),
                    None => v.clone(),
                })
                .collect();
            normalized.insert(field.to_string(), Value::Array(cleaned));
        }
    }

    // Map fields — normalize known placeholder array/object literals,
    // and strip leading `#` from block ID keys.
    for field in &["compress", "shift_to", "split"] {
        if let Some(v) = normalized.get(*field) {
            if let Some(raw) = v.as_str() {
                let trimmed = raw.trim();
                if trimmed == "[]" || trimmed == "{}" {
                    normalized.insert(field.to_string(), Value::Object(serde_json::Map::new()));
                } else if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    if parsed.is_object() {
                        normalized.insert(field.to_string(), parsed);
                    } else if parsed == Value::Array(vec![]) {
                        normalized.insert(field.to_string(), Value::Object(serde_json::Map::new()));
                    }
                }
            } else if v == &Value::Array(vec![]) {
                normalized.insert(field.to_string(), Value::Object(serde_json::Map::new()));
            }
        }

        // Strip `#` prefix from map keys (block IDs)
        if let Some(Value::Object(map)) = normalized.get(*field) {
            let needs_fix = map.keys().any(|k| k.starts_with('#'));
            if needs_fix {
                let cleaned: serde_json::Map<String, Value> = map
                    .iter()
                    .map(|(k, v)| {
                        let key = k.strip_prefix('#').unwrap_or(k).to_string();
                        (key, v.clone())
                    })
                    .collect();
                normalized.insert(field.to_string(), Value::Object(cleaned));
            }
        }
    }
    Value::Object(normalized)
}

/// Known top-level parameter names for the plan tool.
const KNOWN_PLAN_PARAMS: &[&str] = &[
    "archive", "expand", "recall", "pin", "unpin", "shift_to", "compress", "split", "control",
];

/// Check for unrecognized top-level keys in plan arguments.
/// Returns an error message if unknown keys are found.
fn check_unknown_plan_params(arguments: &Value) -> Option<String> {
    let obj = arguments.as_object()?;

    let unknown: Vec<&String> = obj
        .keys()
        .filter(|k| !KNOWN_PLAN_PARAMS.contains(&k.as_str()))
        .collect();

    if unknown.is_empty() {
        return None;
    }

    Some(format!(
        "Unknown parameter(s): {}. Expected parameters:\n\
         - archive: [block_ids] — remove blocks from active context\n\
         - expand: [block_ids] — restore compressed blocks to full content\n\
         - recall: [block_ids] — bring back archived blocks\n\
         - pin: [block_ids] — prevent blocks from being archived\n\
         - unpin: [block_ids] — allow blocks to be archived again\n\
         - shift_to: {{block_id: zone}} — move blocks between zones (primacy/middle/recency)\n\
         - compress: {{block_id: summary}} — replace block content with summary\n\
         - split: {{thread_id: {{at: turn, archive_before: bool}}}} — split thread at turn\n\
         - control: {{op: stage|append|preview|commit|discard}} — plan lifecycle control",
        unknown.iter().map(|k| format!("\"{}\"", k)).collect::<Vec<_>>().join(", ")
    ))
}

/// Plan context changes using a staged strategy workflow.
pub(super) fn context_plan(
    arguments: &Value,
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    session_id: &str,
) -> ToolOutput {
    // Check for unknown parameters before normalization
    if let Some(err_msg) = check_unknown_plan_params(arguments) {
        return ToolOutput::err(err_msg);
    }

    let normalized = normalize_plan_arguments(arguments);
    let actions: PlanActions = match serde_json::from_value(normalized) {
        Ok(a) => a,
        Err(e) => return ToolOutput::err(format!("Invalid plan actions: {e}")),
    };
    let has_actions = has_action_payload(&actions);
    let op = match parse_plan_control_op(arguments, has_actions) {
        Ok(op) => op,
        Err(msg) => return ToolOutput::err(msg),
    };

    match op {
        PlanControlOp::Preview => match planner.staged_plan_for_session(session_id) {
            Some(staged) => ToolOutput::ok(format_plan_preview(&staged, "Staged plan preview")),
            None => ToolOutput::ok(
                "No staged plan yet. Call context_plan with actions (or control.op='stage') to begin strategic staging."
                    .to_string(),
            ),
        },
        PlanControlOp::Discard => {
            let had_staged = planner.clear_staged_plan_for_session(session_id);
            if had_staged {
                ToolOutput::ok(
                    "Discarded staged plan. Autonomous heuristics resume on normal planner turns."
                        .to_string(),
                )
            } else {
                ToolOutput::ok("No staged plan to discard.".to_string())
            }
        }
        PlanControlOp::Commit => match planner.commit_staged_plan_for_session(session_id) {
            Some(staged) => {
                // Eagerly persist archive/recall IDs so they survive even if the
                // pending plan is never consumed by the rewriter (R9-1 fix).
                planner.add_persistent_archives_for_session(session_id, &staged.mutations);

                let mut output = format_plan_preview(&staged, "Committed staged plan");
                output.push_str(
                    "\nCommit queued. These changes apply on the next between-turn rewrite pass. Continue with implementation using the updated context summary.",
                );
                ToolOutput::ok(output)
            }
            None => ToolOutput::err(
                "No staged plan to commit. Stage actions first, then commit.".to_string(),
            ),
        },
        PlanControlOp::Stage | PlanControlOp::Append => {
            if !has_actions {
                return ToolOutput::err(
                    "No plan actions provided. Include actions (archive/pin/shift/etc.) or use control.op='preview'.".to_string(),
                );
            }
            match planner.validate_plan(&actions, blocks, budget) {
                Ok(validated) => {
                    let staged = if op == PlanControlOp::Append {
                        planner.append_staged_plan_for_session(session_id, validated, blocks, budget)
                    } else {
                        planner.set_staged_plan_for_session(session_id, validated.clone());
                        validated
                    };
                    let header = if op == PlanControlOp::Append {
                        "Plan appended to staged strategy"
                    } else {
                        "Plan staged"
                    };
                    ToolOutput::ok(format_plan_preview(&staged, header))
                }
                Err(errors) => {
                    ToolOutput::err(format!("Plan validation failed:\n  {}", errors.join("\n  ")))
                }
            }
        }
    }
}
