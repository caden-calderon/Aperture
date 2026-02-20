//! Context tool implementations (shared across all runtimes).
//!
//! These implement the 5 context management tools:
//! - `context_preview()` — Block inventory with smart-extracted previews
//! - `context_read(id)` — Full content of a single block
//! - `context_search(query, scope)` — Search across active + archived blocks
//! - `context_plan(actions/control)` — Stage/append/preview/commit/discard context plans
//! - `context_status()` — Full detailed manifest on demand

use serde_json::Value;
use std::collections::HashSet;

mod plan;
#[cfg(test)]
mod tests;

use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;
use crate::engine::planner::types::PlanActions;
use crate::engine::planner::ContextPlanner;

use self::plan::{context_plan, normalize_plan_arguments};
use super::preview::{extract_preview, BlockPreview};

/// Result from executing a context tool.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    fn ok(content: String) -> Self {
        Self {
            content,
            is_error: false,
        }
    }

    fn err(msg: String) -> Self {
        Self {
            content: msg,
            is_error: true,
        }
    }
}

/// Output shaping limits for context tools.
#[derive(Debug, Clone, Copy)]
pub struct ToolOutputLimits {
    pub max_output_chars: usize,
    pub max_preview_blocks: usize,
    pub max_search_results: usize,
    pub max_read_chars: usize,
    pub max_status_chars: usize,
}

impl Default for ToolOutputLimits {
    fn default() -> Self {
        Self {
            max_output_chars: 8_000,
            max_preview_blocks: 32,
            max_search_results: 10,
            max_read_chars: 6_000,
            max_status_chars: 8_000,
        }
    }
}

impl ToolOutputLimits {
    /// Aggressive compact mode used under runaway guardrails.
    pub fn compact() -> Self {
        Self {
            max_output_chars: 2_000,
            max_preview_blocks: 12,
            max_search_results: 5,
            max_read_chars: 1_800,
            max_status_chars: 2_000,
        }
    }
}

// ── Tool Dispatcher ──────────────────────────────────────────

pub const DEFAULT_TOOL_SESSION_ID: &str = "__legacy__";

/// Dispatch a context tool call by name.
pub fn dispatch_tool(
    name: &str,
    arguments: &Value,
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
) -> ToolOutput {
    dispatch_tool_for_session(
        name,
        arguments,
        blocks,
        budget,
        planner,
        DEFAULT_TOOL_SESSION_ID,
    )
}

/// Dispatch a context tool call by name for a specific planner session.
pub fn dispatch_tool_for_session(
    name: &str,
    arguments: &Value,
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    session_id: &str,
) -> ToolOutput {
    if let Err(msg) = validate_tool_call(name, arguments, blocks) {
        return ToolOutput::err(msg);
    }

    let output = match name {
        "aperture_context_preview" => context_preview(blocks, budget, planner, session_id),
        "aperture_context_read" => {
            let block_id = arguments
                .get("block_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            context_read(block_id, blocks)
        }
        "aperture_context_search" => {
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let scope = arguments
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("active");
            context_search(query, scope, blocks)
        }
        "aperture_context_plan" => context_plan(arguments, blocks, budget, planner, session_id),
        "aperture_context_status" => context_status(blocks, budget, planner, session_id),
        _ => ToolOutput::err(format!("Unknown tool: {name}")),
    };

    enforce_output_limits(name, output, ToolOutputLimits::default())
}

/// Dispatch a context tool call by name with explicit output shaping limits.
pub fn dispatch_tool_with_limits(
    name: &str,
    arguments: &Value,
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    limits: ToolOutputLimits,
) -> ToolOutput {
    dispatch_tool_with_limits_for_session(
        name,
        arguments,
        blocks,
        budget,
        planner,
        limits,
        DEFAULT_TOOL_SESSION_ID,
    )
}

/// Dispatch a context tool call by name with explicit output shaping limits
/// for a specific planner session.
pub fn dispatch_tool_with_limits_for_session(
    name: &str,
    arguments: &Value,
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    limits: ToolOutputLimits,
    session_id: &str,
) -> ToolOutput {
    if let Err(msg) = validate_tool_call(name, arguments, blocks) {
        return ToolOutput::err(msg);
    }

    let output = match name {
        "aperture_context_preview" => {
            context_preview_with_limits(blocks, budget, planner, limits, session_id)
        }
        "aperture_context_read" => {
            let block_id = arguments
                .get("block_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            context_read_with_limits(block_id, blocks, limits)
        }
        "aperture_context_search" => {
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let scope = arguments
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("active");
            context_search_with_limits(query, scope, blocks, limits)
        }
        "aperture_context_plan" => context_plan(arguments, blocks, budget, planner, session_id),
        "aperture_context_status" => {
            context_status_with_limits(blocks, budget, planner, limits, session_id)
        }
        _ => ToolOutput::err(format!("Unknown tool: {name}")),
    };

    enforce_output_limits(name, output, limits)
}

// ── Tool Implementations ─────────────────────────────────────

/// Preview all blocks with smart-extracted summaries, grouped by zone.
fn context_preview(
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    session_id: &str,
) -> ToolOutput {
    context_preview_with_limits(
        blocks,
        budget,
        planner,
        ToolOutputLimits::default(),
        session_id,
    )
}

fn context_preview_with_limits(
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    limits: ToolOutputLimits,
    session_id: &str,
) -> ToolOutput {
    let show_count = limits.max_preview_blocks.min(blocks.len());
    let previews: Vec<BlockPreview> = blocks
        .iter()
        .take(show_count)
        .map(extract_preview)
        .collect();

    let mut primacy = Vec::new();
    let mut middle = Vec::new();
    let mut recency = Vec::new();
    let mut custom = Vec::new();

    for preview in &previews {
        match preview.zone.as_str() {
            z if z.contains("primacy") => primacy.push(preview),
            z if z.contains("middle") => middle.push(preview),
            z if z.contains("recency") => recency.push(preview),
            _ => custom.push(preview),
        }
    }

    let signals = planner.preview_signals_for_session(session_id, blocks, budget);
    let suggestions = planner.generate_archival_suggestions(blocks, budget, &signals);

    let pct = (budget.utilization * 100.0).round() as u32;
    let mut output = format!(
        "Context Preview - {} blocks, {pct}% budget ({} used / {} limit)\n",
        blocks.len(),
        format_tokens(budget.used_tokens),
        format_tokens(budget.limit_tokens)
    );

    if !suggestions.is_empty() {
        let suggestion_tokens: u32 = suggestions.iter().map(|s| s.tokens).sum();
        output.push_str(&format!(
            "\nArchival Suggestions: {} blocks (~{} tokens) recommended for archival\n",
            suggestions.len(),
            format_tokens(suggestion_tokens)
        ));
        for (idx, suggestion) in suggestions.iter().take(5).enumerate() {
            output.push_str(&format!(
                "  {}. Block #{} - {:?} ({}, {}, {:?} zone, {} tokens)\n",
                idx + 1,
                suggestion.block_id,
                suggestion.tier,
                suggestion.reason,
                if !suggestion.file_refs.is_empty() {
                    suggestion.file_refs.join(", ")
                } else {
                    "no file refs".to_string()
                },
                suggestion.zone,
                suggestion.tokens
            ));
        }
        if suggestions.len() > 5 {
            output.push_str(&format!("  ... and {} more\n", suggestions.len() - 5));
        }
        output.push_str("\nUse aperture_context_plan to review and apply suggestions.\n");
    }

    if !primacy.is_empty() {
        output.push_str(&format_preview_zone("Primacy", &primacy));
    }
    if !middle.is_empty() {
        output.push_str(&format_preview_zone("Middle", &middle));
    }
    if !recency.is_empty() {
        output.push_str(&format_preview_zone("Recency", &recency));
    }
    if !custom.is_empty() {
        output.push_str(&format_preview_zone("Custom", &custom));
    }

    if blocks.len() > show_count {
        output.push_str(&format!(
            "\n... {} additional blocks omitted by Aperture output guardrail.\n",
            blocks.len() - show_count
        ));
    }

    ToolOutput::ok(output)
}

/// Read the full content of a specific block.
fn context_read(block_id: &str, blocks: &[Block]) -> ToolOutput {
    context_read_with_limits(block_id, blocks, ToolOutputLimits::default())
}

fn context_read_with_limits(
    block_id: &str,
    blocks: &[Block],
    limits: ToolOutputLimits,
) -> ToolOutput {
    if block_id.is_empty() {
        return ToolOutput::err("block_id is required".into());
    }

    match blocks.iter().find(|b| b.id == block_id) {
        Some(block) => {
            let role = format!("{:?}", block.role).to_lowercase();
            let zone = format!("{:?}", block.zone).to_lowercase();
            let header = format!(
                "Block #{} - {role} ({zone}, {} tokens, turn {})\n---\n",
                block.id, block.tokens, block.metadata.turn_index
            );
            let body = truncate_chars_with_notice(&block.content, limits.max_read_chars, "read");
            ToolOutput::ok(format!("{header}{body}"))
        }
        None => ToolOutput::err(format!("Block '{block_id}' not found")),
    }
}

/// Search active blocks (and optionally archived) by keyword.
fn context_search(query: &str, scope: &str, blocks: &[Block]) -> ToolOutput {
    context_search_with_limits(query, scope, blocks, ToolOutputLimits::default())
}

fn context_search_with_limits(
    query: &str,
    scope: &str,
    blocks: &[Block],
    limits: ToolOutputLimits,
) -> ToolOutput {
    if query.is_empty() {
        return ToolOutput::err("query is required".into());
    }

    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();

    for block in blocks {
        let score = search_score(block, &query_lower);
        if score > 0 {
            matches.push((block, score));
        }
    }

    matches.sort_by(|a, b| b.1.cmp(&a.1));

    if matches.is_empty() {
        return ToolOutput::ok(format!("No matches for '{query}' in {scope} blocks."));
    }

    let mut output = format!(
        "Search results for '{}' ({} matches in {scope} blocks):\n",
        query,
        matches.len()
    );

    for (block, score) in matches.iter().take(limits.max_search_results) {
        let preview = extract_preview(block);
        let snippet = extract_search_snippet(&block.content, &query_lower, 100);
        output.push_str(&format!(
            "\n  {} (relevance: {score})\n    {snippet}\n",
            preview.to_summary_line()
        ));
    }

    if matches.len() > limits.max_search_results {
        output.push_str(&format!(
            "\n  ... and {} more matches\n",
            matches.len() - limits.max_search_results
        ));
    }

    ToolOutput::ok(output)
}

/// Full detailed context status (manifest).
fn context_status(
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    session_id: &str,
) -> ToolOutput {
    context_status_with_limits(
        blocks,
        budget,
        planner,
        ToolOutputLimits::default(),
        session_id,
    )
}

fn context_status_with_limits(
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
    _limits: ToolOutputLimits,
    _session_id: &str,
) -> ToolOutput {
    let full = planner.generate_full_manifest(blocks, budget);
    ToolOutput::ok(full)
}

fn validate_tool_call(name: &str, arguments: &Value, blocks: &[Block]) -> Result<(), String> {
    if !arguments.is_object() {
        return Err("Tool arguments must be a JSON object.".to_string());
    }

    match name {
        "aperture_context_preview" | "aperture_context_status" => Ok(()),
        "aperture_context_read" => {
            let block_id = arguments
                .get("block_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            if block_id.is_empty() {
                return Err("block_id is required".to_string());
            }
            if block_id.len() > 128 {
                return Err("block_id is too long".to_string());
            }
            if !blocks.iter().any(|b| b.id == block_id) {
                let sample_ids = blocks
                    .iter()
                    .take(8)
                    .map(|b| b.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Block '{block_id}' not found. Call context_preview first and use a valid block ID. Sample IDs: {sample_ids}"
                ));
            }
            Ok(())
        }
        "aperture_context_search" => {
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if query.is_empty() {
                return Err("query is required".to_string());
            }
            if query.chars().count() > 256 {
                return Err("query too long (max 256 chars)".to_string());
            }
            if let Some(scope) = arguments.get("scope").and_then(|v| v.as_str()) {
                if scope != "active" && scope != "all" {
                    return Err("scope must be 'active' or 'all'".to_string());
                }
            }
            Ok(())
        }
        "aperture_context_plan" => {
            let normalized = normalize_plan_arguments(arguments);
            validate_plan_arguments(&normalized, blocks)
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

fn validate_plan_arguments(arguments: &Value, blocks: &[Block]) -> Result<(), String> {
    let actions: PlanActions = serde_json::from_value(arguments.clone())
        .map_err(|e| format!("Invalid plan actions: {e}"))?;

    let total_actions = actions.expand.len()
        + actions.archive.len()
        + actions.recall.len()
        + actions.pin.len()
        + actions.unpin.len()
        + actions.shift_to.len()
        + actions.compress.len()
        + actions.split.len();

    if total_actions > 64 {
        return Err("Plan too large. Limit each plan call to <=64 total mutations.".to_string());
    }

    let known_ids: HashSet<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
    let mut unknown_ids: Vec<String> = Vec::new();

    for block_id in actions
        .expand
        .iter()
        .chain(actions.archive.iter())
        .chain(actions.pin.iter())
        .chain(actions.unpin.iter())
        .chain(actions.shift_to.keys())
        .chain(actions.compress.keys())
    {
        let nid = block_id.strip_prefix('#').unwrap_or(block_id);
        if !known_ids.contains(nid) {
            unknown_ids.push(nid.to_string());
        }
    }

    unknown_ids.sort();
    unknown_ids.dedup();

    if !unknown_ids.is_empty() {
        return Err(format!(
            "Plan references unknown block IDs: {}. Call context_preview first and restage.",
            unknown_ids.join(", ")
        ));
    }

    Ok(())
}

fn enforce_output_limits(
    name: &str,
    mut output: ToolOutput,
    limits: ToolOutputLimits,
) -> ToolOutput {
    let tool_limit = match name {
        "aperture_context_read" => limits.max_read_chars,
        "aperture_context_status" => limits.max_status_chars,
        _ => limits.max_output_chars,
    };

    output.content = truncate_chars_with_notice(&output.content, tool_limit, name);
    output
}

fn truncate_chars_with_notice(content: &str, max_chars: usize, tool_name: &str) -> String {
    let total = content.chars().count();
    if total <= max_chars {
        return content.to_string();
    }

    let reserve = 180usize.min(max_chars / 3);
    let keep = max_chars.saturating_sub(reserve);
    let mut truncated: String = content.chars().take(keep).collect();
    truncated.push_str(&format!(
        "\n\n[Output truncated by Aperture guardrail for {tool_name}: showing {keep}/{total} chars to reduce prompt bloat.]"
    ));
    truncated
}

// ── Helpers ──────────────────────────────────────────────────

/// Score a block's relevance to a search query.
///
/// Tokenizes the query into whitespace-separated terms and scores each
/// independently. Bonus for matching multiple distinct terms.
fn search_score(block: &Block, query_lower: &str) -> u32 {
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    if terms.is_empty() {
        return 0;
    }

    let content_lower = block.content.to_lowercase();
    let role_str = format!("{:?}", block.role).to_lowercase();

    let mut total_score = 0u32;
    let mut terms_matched = 0u32;

    for term in &terms {
        let mut term_score = 0u32;

        let content_matches = content_lower.matches(term).count() as u32;
        term_score += content_matches.min(5) * 2;

        for path in &block.metadata.file_paths {
            if path.to_lowercase().contains(term) {
                term_score += 5;
            }
        }

        if role_str.contains(term) {
            term_score += 3;
        }

        if let Some(ref tool) = block.metadata.tool_name {
            if tool.to_lowercase().contains(term) {
                term_score += 4;
            }
        }

        for kw in &block.topic_keywords {
            if kw.to_lowercase().contains(term) {
                term_score += 3;
            }
        }

        if term_score > 0 {
            terms_matched += 1;
        }
        total_score += term_score;
    }

    // Bonus for matching multiple distinct terms (multi-term relevance)
    if terms_matched > 1 {
        total_score += terms_matched * 3;
    }

    total_score
}

/// Extract a search snippet around the first matching query term.
fn extract_search_snippet(content: &str, query_lower: &str, max_len: usize) -> String {
    let content_lower = content.to_lowercase();

    // Find the first matching term position
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    let best_pos = terms
        .iter()
        .filter_map(|term| content_lower.find(term))
        .min();

    if let Some(pos) = best_pos {
        let match_term_len = terms
            .iter()
            .find(|t| content_lower[pos..].starts_with(*t))
            .map(|t| t.len())
            .unwrap_or(query_lower.len());

        let start = pos.saturating_sub(40);
        let end = (pos + match_term_len + 60).min(content.len());

        let safe_start = content
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= start)
            .unwrap_or(0);
        let safe_end = content
            .char_indices()
            .map(|(i, _)| i)
            .rfind(|&i| i <= end)
            .unwrap_or(content.len());

        let snippet = &content[safe_start..safe_end];
        let snippet = snippet.replace('\n', " ");

        if snippet.len() > max_len {
            format!("...{}...", &snippet[..max_len])
        } else {
            let prefix = if safe_start > 0 { "..." } else { "" };
            let suffix = if safe_end < content.len() { "..." } else { "" };
            format!("{prefix}{snippet}{suffix}")
        }
    } else {
        content
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(max_len)
            .collect()
    }
}

/// Format a preview zone section.
fn format_preview_zone(name: &str, previews: &[&BlockPreview]) -> String {
    let total_tokens: u32 = previews.iter().map(|p| p.tokens).sum();
    let mut section = format!(
        "\n[{name}] ({} blocks, {})\n",
        previews.len(),
        format_tokens(total_tokens)
    );

    for preview in previews {
        section.push_str(&format!("  {}\n", preview.to_summary_line()));
        if !preview.head.is_empty() {
            let first_line = preview.head.lines().next().unwrap_or("");
            if !first_line.is_empty() {
                let truncated: String = first_line.chars().take(80).collect();
                section.push_str(&format!("    > {truncated}\n"));
            }
        }
    }

    section
}

/// Describe a mutation in human-readable form.
fn describe_mutation(mutation: &crate::engine::planner::types::ContextMutation) -> String {
    use crate::engine::planner::types::ContextMutation;
    match mutation {
        ContextMutation::Expand { block_id } => format!("Expand #{block_id} to full content"),
        ContextMutation::Archive { block_id } => {
            format!("Archive #{block_id} (remove from payload)")
        }
        ContextMutation::Recall { block_id } => {
            format!("Recall #{block_id} into active context")
        }
        ContextMutation::Pin { block_id } => format!("Pin #{block_id}"),
        ContextMutation::Unpin { block_id } => format!("Unpin #{block_id}"),
        ContextMutation::Shift {
            block_id,
            target_zone,
        } => format!("Shift #{block_id} -> {target_zone:?}"),
        ContextMutation::Compress {
            block_id, summary, ..
        } => {
            let preview: String = summary.chars().take(50).collect();
            format!("Compress #{block_id}: \"{preview}...\"")
        }
        ContextMutation::Split {
            thread_id, at_turn, ..
        } => format!("Split thread {thread_id} at turn {at_turn}"),
        ContextMutation::UpdateContent { block_id, .. } => {
            format!("Update content of #{block_id}")
        }
    }
}

/// Format token count for display.
fn format_tokens(tokens: u32) -> String {
    if tokens >= 1000 {
        let k = tokens as f64 / 1000.0;
        if k >= 10.0 {
            format!("{}k", k.round() as u32)
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{tokens}")
    }
}
