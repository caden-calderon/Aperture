//! Context tool implementations (shared across all runtimes).
//!
//! These implement the 5 context management tools:
//! - `context_preview()` — Block inventory with smart-extracted previews
//! - `context_read(id)` — Full content of a single block
//! - `context_search(query, scope)` — Search across active + archived blocks
//! - `context_plan(actions)` — Plan changes, return preview (last-plan-wins)
//! - `context_status()` — Full detailed manifest on demand

use serde_json::Value;

use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;
use crate::engine::planner::types::PlanActions;
use crate::engine::planner::ContextPlanner;

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

// ── Tool Dispatcher ──────────────────────────────────────────

/// Dispatch a context tool call by name.
pub fn dispatch_tool(
    name: &str,
    arguments: &Value,
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
) -> ToolOutput {
    match name {
        "aperture_context_preview" => context_preview(blocks, budget),
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
        "aperture_context_plan" => context_plan(arguments, blocks, budget, planner),
        "aperture_context_status" => context_status(blocks, budget, planner),
        _ => ToolOutput::err(format!("Unknown tool: {name}")),
    }
}

// ── Tool Implementations ─────────────────────────────────────

/// Preview all blocks with smart-extracted summaries, grouped by zone.
fn context_preview(blocks: &[Block], budget: &BudgetStatus) -> ToolOutput {
    let previews: Vec<BlockPreview> = blocks.iter().map(extract_preview).collect();

    // Group by zone
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

    let pct = (budget.utilization * 100.0).round() as u32;
    let mut output = format!(
        "Context Preview — {} blocks, {pct}% budget ({} used / {} limit)\n",
        blocks.len(),
        format_tokens(budget.used_tokens),
        format_tokens(budget.limit_tokens)
    );

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

    ToolOutput::ok(output)
}

/// Read the full content of a specific block.
fn context_read(block_id: &str, blocks: &[Block]) -> ToolOutput {
    if block_id.is_empty() {
        return ToolOutput::err("block_id is required".into());
    }

    match blocks.iter().find(|b| b.id == block_id) {
        Some(block) => {
            let role = format!("{:?}", block.role).to_lowercase();
            let zone = format!("{:?}", block.zone).to_lowercase();
            let header = format!(
                "Block #{} — {role} ({zone}, {} tokens, turn {})\n---\n",
                block.id, block.tokens, block.metadata.turn_index
            );
            ToolOutput::ok(format!("{header}{}", block.content))
        }
        None => ToolOutput::err(format!("Block '{block_id}' not found")),
    }
}

/// Search active blocks (and optionally archived) by keyword.
fn context_search(query: &str, scope: &str, blocks: &[Block]) -> ToolOutput {
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

    // Sort by relevance score (descending)
    matches.sort_by(|a, b| b.1.cmp(&a.1));

    if matches.is_empty() {
        return ToolOutput::ok(format!("No matches for '{query}' in {scope} blocks."));
    }

    let mut output = format!(
        "Search results for '{}' ({} matches in {scope} blocks):\n",
        query,
        matches.len()
    );

    for (block, score) in matches.iter().take(10) {
        let preview = extract_preview(block);
        let snippet = extract_search_snippet(&block.content, &query_lower, 100);
        output.push_str(&format!(
            "\n  {} (relevance: {score})\n    {snippet}\n",
            preview.to_summary_line()
        ));
    }

    if matches.len() > 10 {
        output.push_str(&format!(
            "\n  ... and {} more matches\n",
            matches.len() - 10
        ));
    }

    ToolOutput::ok(output)
}

/// Plan context changes. Validates actions, returns preview. Last-plan-wins.
fn context_plan(
    arguments: &Value,
    blocks: &[Block],
    budget: &BudgetStatus,
    planner: &ContextPlanner,
) -> ToolOutput {
    // Parse the actions
    let actions: PlanActions = match serde_json::from_value(arguments.clone()) {
        Ok(a) => a,
        Err(e) => return ToolOutput::err(format!("Invalid plan actions: {e}")),
    };

    // Validate against current state
    match planner.validate_plan(&actions, blocks, budget) {
        Ok(plan) => {
            let mut output = format!("Plan validated — {} mutations:\n", plan.mutations.len());

            for mutation in &plan.mutations {
                output.push_str(&format!("  • {}\n", describe_mutation(mutation)));
            }

            let delta_str = if plan.token_delta > 0 {
                format!("+{}", format_tokens(plan.token_delta as u32))
            } else if plan.token_delta < 0 {
                format!("-{}", format_tokens((-plan.token_delta) as u32))
            } else {
                "no change".into()
            };

            output.push_str(&format!(
                "\nProjected impact: {delta_str} tokens, {} blocks, {:.0}% budget\n",
                plan.projected_block_count,
                plan.projected_utilization * 100.0
            ));
            output.push_str("Changes will be applied between turns.");

            // Store as pending plan (last-plan-wins)
            planner.set_pending_plan(plan);

            ToolOutput::ok(output)
        }
        Err(errors) => ToolOutput::err(format!(
            "Plan validation failed:\n  {}",
            errors.join("\n  ")
        )),
    }
}

/// Full detailed context status (manifest).
fn context_status(blocks: &[Block], budget: &BudgetStatus, planner: &ContextPlanner) -> ToolOutput {
    let full = planner.generate_full_manifest(blocks, budget);
    ToolOutput::ok(full)
}

// ── Helpers ──────────────────────────────────────────────────

/// Score a block's relevance to a search query.
fn search_score(block: &Block, query_lower: &str) -> u32 {
    let mut score = 0u32;

    let content_lower = block.content.to_lowercase();
    let role_str = format!("{:?}", block.role).to_lowercase();

    // Content match (weighted by frequency, capped)
    let content_matches = content_lower.matches(query_lower).count() as u32;
    score += content_matches.min(5) * 2;

    // File path match (high value)
    for path in &block.metadata.file_paths {
        if path.to_lowercase().contains(query_lower) {
            score += 5;
        }
    }

    // Role match
    if role_str.contains(query_lower) {
        score += 3;
    }

    // Tool name match
    if let Some(ref tool) = block.metadata.tool_name {
        if tool.to_lowercase().contains(query_lower) {
            score += 4;
        }
    }

    // Topic keyword match
    for kw in &block.topic_keywords {
        if kw.to_lowercase().contains(query_lower) {
            score += 3;
        }
    }

    score
}

/// Extract a search snippet around the query match.
fn extract_search_snippet(content: &str, query_lower: &str, max_len: usize) -> String {
    let content_lower = content.to_lowercase();

    if let Some(pos) = content_lower.find(query_lower) {
        let start = pos.saturating_sub(40);
        let end = (pos + query_lower.len() + 60).min(content.len());

        // Ensure we don't slice in the middle of a multi-byte char
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
        // No match in content, show first line
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
            // Show first line of head for context
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
        } => format!("Shift #{block_id} → {target_zone:?}"),
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

#[cfg(test)]
mod tests {
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
        BudgetStatus {
            used_tokens: used,
            limit_tokens: limit,
            utilization: if limit > 0 {
                used as f64 / limit as f64
            } else {
                0.0
            },
            alert_level: AlertLevel::Normal,
            remaining_tokens: limit.saturating_sub(used),
        }
    }

    #[test]
    fn test_context_preview_basic() {
        let blocks = vec![
            make_block(
                "1",
                Role::System,
                BuiltInZone::Primacy,
                500,
                "You are a helpful assistant.",
            ),
            make_block(
                "2",
                Role::User,
                BuiltInZone::Recency,
                200,
                "Help me with this code.",
            ),
            make_block(
                "3",
                Role::Assistant,
                BuiltInZone::Recency,
                800,
                "pub fn solve() { todo!() }",
            ),
        ];
        let budget = mock_budget(90_000, 200_000);

        let output = context_preview(&blocks, &budget);
        assert!(!output.is_error);
        assert!(output.content.contains("3 blocks"));
        assert!(output.content.contains("45%"));
        assert!(output.content.contains("Primacy"));
        assert!(output.content.contains("Recency"));
    }

    #[test]
    fn test_context_read_found() {
        let blocks = vec![make_block(
            "42",
            Role::User,
            BuiltInZone::Recency,
            100,
            "This is the full content.",
        )];

        let output = context_read("42", &blocks);
        assert!(!output.is_error);
        assert!(output.content.contains("Block #42"));
        assert!(output.content.contains("This is the full content."));
    }

    #[test]
    fn test_context_read_not_found() {
        let blocks = vec![];
        let output = context_read("999", &blocks);
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[test]
    fn test_context_search_matches() {
        let blocks = vec![
            make_block(
                "1",
                Role::User,
                BuiltInZone::Recency,
                100,
                "Fix the authentication bug",
            ),
            make_block(
                "2",
                Role::Assistant,
                BuiltInZone::Recency,
                200,
                "The auth module has a token validation issue",
            ),
            make_block(
                "3",
                Role::User,
                BuiltInZone::Recency,
                100,
                "Now let's work on the database",
            ),
        ];

        let output = context_search("auth", "active", &blocks);
        assert!(!output.is_error);
        assert!(output.content.contains("2 matches"));
        // Both "auth" blocks should match
        assert!(output.content.contains("#1") || output.content.contains("#2"));
    }

    #[test]
    fn test_context_search_no_matches() {
        let blocks = vec![make_block(
            "1",
            Role::User,
            BuiltInZone::Recency,
            100,
            "Hello world",
        )];

        let output = context_search("nonexistent", "active", &blocks);
        assert!(!output.is_error);
        assert!(output.content.contains("No matches"));
    }

    #[test]
    fn test_context_plan_valid() {
        let blocks = vec![
            make_block("1", Role::User, BuiltInZone::Recency, 500, "User message"),
            make_block(
                "2",
                Role::Assistant,
                BuiltInZone::Middle,
                1000,
                "Old assistant response",
            ),
        ];
        let budget = mock_budget(50_000, 200_000);
        let planner = ContextPlanner::with_default_config();

        let args = serde_json::json!({
            "archive": ["2"],
            "pin": ["1"]
        });

        let output = context_plan(&args, &blocks, &budget, &planner);
        assert!(!output.is_error);
        assert!(output.content.contains("2 mutations"));
        assert!(output.content.contains("Archive #2"));
        assert!(output.content.contains("Pin #1"));
        assert!(output.content.contains("applied between turns"));

        // Verify plan was stored
        assert!(planner.has_pending_plan());
    }

    #[test]
    fn test_context_plan_invalid_block() {
        let blocks = vec![];
        let budget = mock_budget(50_000, 200_000);
        let planner = ContextPlanner::with_default_config();

        let args = serde_json::json!({
            "archive": ["nonexistent"]
        });

        let output = context_plan(&args, &blocks, &budget, &planner);
        assert!(output.is_error);
        assert!(output.content.contains("validation failed"));
    }

    #[test]
    fn test_context_status() {
        let blocks = vec![
            make_block(
                "1",
                Role::System,
                BuiltInZone::Primacy,
                500,
                "System prompt",
            ),
            make_block("2", Role::User, BuiltInZone::Recency, 200, "User question"),
        ];
        let budget = mock_budget(100_000, 200_000);
        let planner = ContextPlanner::with_default_config();

        let output = context_status(&blocks, &budget, &planner);
        assert!(!output.is_error);
        assert!(output.content.contains("Aperture Context Manifest"));
        assert!(output.content.contains("50%"));
    }

    #[test]
    fn test_dispatch_unknown_tool() {
        let blocks = vec![];
        let budget = mock_budget(0, 200_000);
        let planner = ContextPlanner::with_default_config();

        let output = dispatch_tool(
            "unknown_tool",
            &serde_json::json!({}),
            &blocks,
            &budget,
            &planner,
        );
        assert!(output.is_error);
        assert!(output.content.contains("Unknown tool"));
    }

    #[test]
    fn test_search_score_content_match() {
        let block = make_block(
            "1",
            Role::User,
            BuiltInZone::Recency,
            100,
            "authentication auth module",
        );
        let score = search_score(&block, "auth");
        assert!(score > 0);
    }

    #[test]
    fn test_search_snippet_extraction() {
        let content = "This is a long piece of text about authentication and security measures that should be properly handled.";
        let snippet = extract_search_snippet(content, "authentication", 80);
        assert!(snippet.contains("authentication"));
    }
}
