//! Manifest generation for context awareness injection.
//!
//! Generates tiered manifest text that scales with need:
//! - StatusLine (~30 tok): budget %, block counts — always present
//! - Delta (~50-100 tok): what changed since last turn — on change
//! - Full (~200-300 tok): complete block inventory with previews — on demand
//! - Warning (~100-150 tok): budget pressure with recommendations — at thresholds

use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;
use crate::engine::types::{BuiltInZone, CompressionLevel, Zone};

use super::types::{ContextMutation, Manifest};

/// Which manifest level to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestLevel {
    /// Always-present status line (~30 tokens).
    StatusLine,
    /// What changed since last turn (~50-100 tokens).
    Delta,
    /// Complete block inventory (~200-300 tokens).
    Full,
    /// Budget pressure warning (~100-150 tokens).
    Warning,
}

/// Snapshot of changes applied since the last turn, used for delta generation.
#[derive(Debug, Clone, Default)]
pub struct TurnDelta {
    pub archived_ids: Vec<String>,
    pub recalled_ids: Vec<String>,
    pub compressed_ids: Vec<String>,
    pub shifted_ids: Vec<(String, String)>, // (block_id, target_zone_name)
    pub expanded_ids: Vec<String>,
    pub updated_ids: Vec<String>,
    pub net_token_delta: i64,
}

impl TurnDelta {
    pub fn is_empty(&self) -> bool {
        self.archived_ids.is_empty()
            && self.recalled_ids.is_empty()
            && self.compressed_ids.is_empty()
            && self.shifted_ids.is_empty()
            && self.expanded_ids.is_empty()
            && self.updated_ids.is_empty()
    }

    /// Build a TurnDelta from a list of applied mutations.
    pub fn from_mutations(mutations: &[ContextMutation], net_token_delta: i64) -> Self {
        let mut delta = Self {
            net_token_delta,
            ..Default::default()
        };
        for m in mutations {
            match m {
                ContextMutation::Archive { block_id } => {
                    delta.archived_ids.push(block_id.clone());
                }
                ContextMutation::Recall { block_id } => {
                    delta.recalled_ids.push(block_id.clone());
                }
                ContextMutation::Compress { block_id, .. } => {
                    delta.compressed_ids.push(block_id.clone());
                }
                ContextMutation::Shift {
                    block_id,
                    target_zone,
                } => {
                    delta
                        .shifted_ids
                        .push((block_id.clone(), format!("{target_zone:?}").to_lowercase()));
                }
                ContextMutation::Expand { block_id } => {
                    delta.expanded_ids.push(block_id.clone());
                }
                ContextMutation::UpdateContent { block_id, .. } => {
                    delta.updated_ids.push(block_id.clone());
                }
                ContextMutation::Pin { .. }
                | ContextMutation::Unpin { .. }
                | ContextMutation::Split { .. } => {}
            }
        }
        delta
    }
}

// ── Generation Functions ─────────────────────────────────────

/// Generate the always-present status line (~30 tokens).
pub fn generate_status_line(blocks: &[Block], budget: &BudgetStatus) -> String {
    let total = blocks.len();
    let archived_count = 0_usize; // archived blocks aren't in the active list
    let compressed = blocks
        .iter()
        .filter(|b| b.compression_level != CompressionLevel::Original)
        .count();
    let pinned = blocks.iter().filter(|b| b.pinned.is_some()).count();

    let pct = (budget.utilization * 100.0).round() as u32;

    let mut parts = vec![format!(
        "[Aperture: {pct}% budget | {total} blocks | {} remaining",
        format_tokens(budget.remaining_tokens)
    )];

    if compressed > 0 {
        parts.push(format!("{compressed} compressed"));
    }
    if pinned > 0 {
        parts.push(format!("{pinned} pinned"));
    }
    if archived_count > 0 {
        parts.push(format!("{archived_count} archived"));
    }

    // Close the bracket
    let mut line = parts.join(" | ");
    line.push(']');
    line
}

/// Generate delta text showing what changed since last turn (~50-100 tokens).
pub fn generate_delta(delta: &TurnDelta, budget: &BudgetStatus) -> Option<String> {
    if delta.is_empty() {
        return None;
    }

    let mut actions = Vec::new();

    if !delta.archived_ids.is_empty() {
        actions.push(format!("archived {}", format_id_list(&delta.archived_ids)));
    }
    if !delta.recalled_ids.is_empty() {
        actions.push(format!("recalled {}", format_id_list(&delta.recalled_ids)));
    }
    if !delta.compressed_ids.is_empty() {
        actions.push(format!(
            "compressed {}",
            format_id_list(&delta.compressed_ids)
        ));
    }
    if !delta.expanded_ids.is_empty() {
        actions.push(format!("expanded {}", format_id_list(&delta.expanded_ids)));
    }
    if !delta.shifted_ids.is_empty() {
        let shifts: Vec<String> = delta
            .shifted_ids
            .iter()
            .map(|(id, zone)| format!("#{id} → {zone}"))
            .collect();
        actions.push(format!("shifted {}", shifts.join(", ")));
    }
    if !delta.updated_ids.is_empty() {
        actions.push(format!("updated {}", format_id_list(&delta.updated_ids)));
    }

    let token_str = if delta.net_token_delta > 0 {
        format!("+{}", format_tokens(delta.net_token_delta as u32))
    } else if delta.net_token_delta < 0 {
        format!("-{}", format_tokens((-delta.net_token_delta) as u32))
    } else {
        "net 0".into()
    };

    let pct = (budget.utilization * 100.0).round() as u32;

    Some(format!(
        "[Context update: {}. {token_str}. Budget: {pct}%]",
        actions.join(", ")
    ))
}

/// Generate the full manifest with complete block inventory (~200-300 tokens).
pub fn generate_full(blocks: &[Block], budget: &BudgetStatus) -> String {
    let mut sections = Vec::new();

    // Header
    let pct = (budget.utilization * 100.0).round() as u32;
    sections.push(format!(
        "[Aperture Context Manifest — {pct}% budget ({}/{} tokens)]",
        format_tokens(budget.used_tokens),
        format_tokens(budget.limit_tokens)
    ));

    // Group blocks by zone
    let mut primacy = Vec::new();
    let mut middle = Vec::new();
    let mut recency = Vec::new();
    let mut custom = Vec::new();

    for block in blocks {
        match &block.zone {
            Zone::BuiltIn(BuiltInZone::Primacy) => primacy.push(block),
            Zone::BuiltIn(BuiltInZone::Middle) => middle.push(block),
            Zone::BuiltIn(BuiltInZone::Recency) => recency.push(block),
            Zone::Custom(_) => custom.push(block),
        }
    }

    // Render each zone
    if !primacy.is_empty() {
        sections.push(format_zone_section("Primacy", &primacy));
    }
    if !middle.is_empty() {
        sections.push(format_zone_section("Middle", &middle));
    }
    if !recency.is_empty() {
        sections.push(format_zone_section("Recency", &recency));
    }
    if !custom.is_empty() {
        sections.push(format_zone_section("Custom", &custom));
    }

    sections.join("\n")
}

/// Generate a budget warning with recommendations (~100-150 tokens).
pub fn generate_warning(budget: &BudgetStatus, blocks: &[Block]) -> Option<String> {
    let pct = (budget.utilization * 100.0).round() as u32;

    // Only warn above 70%
    if pct < 70 {
        return None;
    }

    let stale_candidates: Vec<&Block> = blocks
        .iter()
        .filter(|b| b.pinned.is_none() && b.zone == Zone::BuiltIn(BuiltInZone::Middle))
        .collect();

    let stale_tokens: u32 = stale_candidates.iter().map(|b| b.tokens).sum();

    let severity = if pct >= 95 {
        "CRITICAL"
    } else if pct >= 85 {
        "HIGH"
    } else {
        "MODERATE"
    };

    let mut warning = format!(
        "[Aperture Budget Warning ({severity}): {pct}% used — {} remaining]",
        format_tokens(budget.remaining_tokens)
    );

    if !stale_candidates.is_empty() {
        warning.push_str(&format!(
            "\n[{} middle-zone blocks ({}) available for archival. Use aperture_context_plan() to manage.]",
            stale_candidates.len(),
            format_tokens(stale_tokens)
        ));
    } else {
        warning.push_str("\n[No easy archival candidates. Consider compressing active blocks.]");
    }

    Some(warning)
}

/// Build a complete Manifest from engine state and optional delta.
pub fn build_manifest(
    blocks: &[Block],
    budget: &BudgetStatus,
    delta: Option<&TurnDelta>,
) -> Manifest {
    let status_line = generate_status_line(blocks, budget);
    let delta_text = delta.and_then(|d| generate_delta(d, budget));
    let warning = generate_warning(budget, blocks);

    Manifest {
        status_line,
        delta: delta_text,
        warning,
    }
}

// ── Helpers ──────────────────────────────────────────────────

/// Format a token count for display (e.g. "1.2k", "45k", "200k").
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

/// Format a list of block IDs for display.
fn format_id_list(ids: &[String]) -> String {
    if ids.len() <= 3 {
        ids.iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        let first_two: Vec<String> = ids.iter().take(2).map(|id| format!("#{id}")).collect();
        format!("{} +{} more", first_two.join(", "), ids.len() - 2)
    }
}

/// Format a zone section for the full manifest.
fn format_zone_section(zone_name: &str, blocks: &[&Block]) -> String {
    let total_tokens: u32 = blocks.iter().map(|b| b.tokens).sum();
    let mut lines = vec![format!(
        "  {zone_name} ({} blocks, {}):",
        blocks.len(),
        format_tokens(total_tokens)
    )];

    for block in blocks {
        let role = format!("{:?}", block.role).to_lowercase();
        let compressed = if block.compression_level != CompressionLevel::Original {
            format!(
                " [{}]",
                format!("{:?}", block.compression_level).to_lowercase()
            )
        } else {
            String::new()
        };
        let pinned = if block.pinned.is_some() { " 📌" } else { "" };
        let preview = truncate_preview(&block.content, 60);
        lines.push(format!(
            "    #{} {role}{compressed}{pinned} ({}) — {preview}",
            block.id,
            format_tokens(block.tokens)
        ));
    }

    lines.join("\n")
}

/// Truncate content to a short preview string.
fn truncate_preview(content: &str, max_chars: usize) -> String {
    let first_line = content.lines().next().unwrap_or("");
    if first_line.len() <= max_chars {
        first_line.to_string()
    } else {
        format!("{}...", &first_line[..max_chars])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{PinPosition, Role};

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
        BudgetStatus {
            used_tokens: used,
            limit_tokens: limit,
            utilization: if limit > 0 {
                used as f64 / limit as f64
            } else {
                0.0
            },
            alert_level: crate::engine::budget::AlertLevel::Normal,
            remaining_tokens: limit.saturating_sub(used),
        }
    }

    #[test]
    fn test_status_line_basic() {
        let blocks = vec![
            mock_block("1", Role::System, BuiltInZone::Primacy, 500),
            mock_block("2", Role::User, BuiltInZone::Recency, 200),
            mock_block("3", Role::Assistant, BuiltInZone::Recency, 300),
        ];
        let budget = mock_budget(90_000, 200_000);

        let line = generate_status_line(&blocks, &budget);
        assert!(line.starts_with("[Aperture:"));
        assert!(line.contains("45%"));
        assert!(line.contains("3 blocks"));
        assert!(line.contains("110k remaining"));
        assert!(line.ends_with(']'));
    }

    #[test]
    fn test_status_line_with_compressed_and_pinned() {
        let mut blocks = vec![
            mock_block("1", Role::System, BuiltInZone::Primacy, 500),
            mock_block("2", Role::User, BuiltInZone::Middle, 200),
        ];
        blocks[0].pinned = Some(PinPosition::Top);
        blocks[1].compression_level = CompressionLevel::Summarized;

        let budget = mock_budget(50_000, 200_000);
        let line = generate_status_line(&blocks, &budget);
        assert!(line.contains("1 compressed"));
        assert!(line.contains("1 pinned"));
    }

    #[test]
    fn test_delta_generation_empty() {
        let delta = TurnDelta::default();
        let budget = mock_budget(50_000, 200_000);
        assert!(generate_delta(&delta, &budget).is_none());
    }

    #[test]
    fn test_delta_generation_with_changes() {
        let delta = TurnDelta {
            archived_ids: vec!["12".into()],
            expanded_ids: vec!["8".into()],
            net_token_delta: -1960,
            ..Default::default()
        };
        let budget = mock_budget(104_000, 200_000);
        let text = generate_delta(&delta, &budget).expect("should generate");
        assert!(text.contains("archived #12"));
        assert!(text.contains("expanded #8"));
        assert!(text.contains("-2.0k")); // 1960 rounds to 2.0k
        assert!(text.contains("52%"));
    }

    #[test]
    fn test_full_manifest_structure() {
        let blocks = vec![
            mock_block("1", Role::System, BuiltInZone::Primacy, 500),
            mock_block("2", Role::User, BuiltInZone::Middle, 1200),
            mock_block("3", Role::Assistant, BuiltInZone::Recency, 800),
        ];
        let budget = mock_budget(100_000, 200_000);

        let full = generate_full(&blocks, &budget);
        assert!(full.contains("Aperture Context Manifest"));
        assert!(full.contains("50%"));
        assert!(full.contains("Primacy"));
        assert!(full.contains("Middle"));
        assert!(full.contains("Recency"));
        assert!(full.contains("#1"));
        assert!(full.contains("#2"));
        assert!(full.contains("#3"));
    }

    #[test]
    fn test_warning_generation_below_threshold() {
        let blocks = vec![mock_block("1", Role::User, BuiltInZone::Recency, 500)];
        let budget = mock_budget(50_000, 200_000); // 25%
        assert!(generate_warning(&budget, &blocks).is_none());
    }

    #[test]
    fn test_warning_generation_above_threshold() {
        let blocks = vec![
            mock_block("1", Role::User, BuiltInZone::Middle, 5000),
            mock_block("2", Role::Assistant, BuiltInZone::Middle, 3000),
        ];
        let budget = mock_budget(180_000, 200_000); // 90%

        let warning = generate_warning(&budget, &blocks).expect("should generate");
        assert!(warning.contains("HIGH"));
        assert!(warning.contains("90%"));
        assert!(warning.contains("2 middle-zone blocks"));
        assert!(warning.contains("aperture_context_plan()"));
    }

    #[test]
    fn test_turn_delta_from_mutations() {
        let mutations = vec![
            ContextMutation::Archive {
                block_id: "b1".into(),
            },
            ContextMutation::Compress {
                block_id: "b2".into(),
                summary: "summary".into(),
            },
            ContextMutation::Shift {
                block_id: "b3".into(),
                target_zone: BuiltInZone::Primacy,
            },
        ];
        let delta = TurnDelta::from_mutations(&mutations, -2000);
        assert_eq!(delta.archived_ids, vec!["b1"]);
        assert_eq!(delta.compressed_ids, vec!["b2"]);
        assert_eq!(delta.shifted_ids.len(), 1);
        assert_eq!(delta.net_token_delta, -2000);
    }

    #[test]
    fn test_build_manifest_combines_levels() {
        let blocks = vec![
            mock_block("1", Role::System, BuiltInZone::Primacy, 500),
            mock_block("2", Role::User, BuiltInZone::Middle, 3000),
        ];
        let budget = mock_budget(160_000, 200_000); // 80% — should trigger warning

        let delta = TurnDelta {
            archived_ids: vec!["5".into()],
            net_token_delta: -1000,
            ..Default::default()
        };

        let manifest = build_manifest(&blocks, &budget, Some(&delta));
        assert!(!manifest.status_line.is_empty());
        assert!(manifest.delta.is_some());
        assert!(manifest.warning.is_some());
        assert!(manifest.has_changes());
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1500), "1.5k");
        assert_eq!(format_tokens(10000), "10k");
        assert_eq!(format_tokens(200000), "200k");
    }

    #[test]
    fn test_format_id_list_short() {
        let ids = vec!["a".into(), "b".into()];
        assert_eq!(format_id_list(&ids), "#a, #b");
    }

    #[test]
    fn test_format_id_list_long() {
        let ids: Vec<String> = (1..=5).map(|i| i.to_string()).collect();
        let formatted = format_id_list(&ids);
        assert!(formatted.contains("+3 more"));
    }
}
