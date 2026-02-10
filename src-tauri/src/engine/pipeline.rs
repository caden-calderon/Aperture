//! Context classification pipeline.
//!
//! Three-pass deterministic classification:
//! 1. Role-based zone assignment (instant)
//! 2. Content heuristics (fast string matching)
//! 3. Budget-aware ranking (staleness-based pruning candidates)

use super::block::Block;
use super::budget::{check_budget, AlertLevel, BudgetConfig};
use super::staleness::{calculate_staleness, StalenessConfig};
use super::zone::{assign_zones, ZoneConfig};

/// Content heuristic classification results.
#[derive(Debug, Clone)]
pub struct HeuristicResult {
    pub block_id: String,
    /// Positive = boost toward recency, negative = demote.
    pub recency_boost: f64,
    /// Flags detected in content.
    pub flags: Vec<ContentFlag>,
}

/// Content-based flags detected by heuristics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentFlag {
    /// Block contains error/traceback content.
    ErrorContent,
    /// Block contains a correction pattern ("actually", "wait", "I meant").
    CorrectionPattern,
    /// Block references a file path.
    FileReference,
    /// Block contains code.
    CodeContent,
    /// Block is very short (< 20 tokens).
    VeryShort,
    /// Block is very large (> 2000 tokens).
    VeryLarge,
}

/// Pipeline configuration.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub zone_config: ZoneConfig,
    pub staleness_config: StalenessConfig,
    pub budget_config: BudgetConfig,
    pub token_limit: u32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            zone_config: ZoneConfig::default(),
            staleness_config: StalenessConfig::default(),
            budget_config: BudgetConfig::default(),
            token_limit: 200_000,
        }
    }
}

/// Run the full classification pipeline on a set of blocks.
///
/// Mutates blocks in-place (zone assignment, heat updates).
/// Returns pruning candidates if over budget.
pub fn classify(blocks: &mut [Block], config: &PipelineConfig) -> ClassificationResult {
    // Pass 1: Role-based zone assignment
    assign_zones(blocks, &config.zone_config);

    // Pass 2: Content heuristics
    let heuristics: Vec<HeuristicResult> = blocks.iter().map(run_heuristics).collect();

    // Apply heuristic boosts to block heat
    for (block, result) in blocks.iter_mut().zip(heuristics.iter()) {
        // Error content and corrections boost relevance
        if result.flags.contains(&ContentFlag::ErrorContent)
            || result.flags.contains(&ContentFlag::CorrectionPattern)
        {
            block.usage_heat = (block.usage_heat + 0.2).min(1.0);
        }
    }

    // Pass 3: Budget-aware ranking
    let total_tokens: u32 = blocks.iter().map(|b| b.tokens).sum();
    let alert = check_budget(total_tokens, config.token_limit, &config.budget_config);

    let pruning_candidates = if alert >= AlertLevel::Warning {
        let current_turn = blocks
            .iter()
            .map(|b| b.metadata.turn_index)
            .max()
            .unwrap_or(0);

        let mut candidates: Vec<PruningCandidate> = blocks
            .iter()
            .filter(|b| b.pinned.is_none()) // never suggest pruning pinned blocks
            .map(|b| {
                let staleness = calculate_staleness(b, current_turn + 1, &config.staleness_config);
                PruningCandidate {
                    block_id: b.id.clone(),
                    staleness,
                    tokens: b.tokens,
                    reason: pruning_reason(b, staleness),
                }
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.staleness
                .partial_cmp(&a.staleness)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates
    } else {
        vec![]
    };

    ClassificationResult {
        total_tokens,
        alert_level: alert,
        heuristics,
        pruning_candidates,
    }
}

/// Result of a full pipeline classification run.
#[derive(Debug, Clone)]
pub struct ClassificationResult {
    pub total_tokens: u32,
    pub alert_level: AlertLevel,
    pub heuristics: Vec<HeuristicResult>,
    pub pruning_candidates: Vec<PruningCandidate>,
}

/// A block suggested for pruning/compression.
#[derive(Debug, Clone)]
pub struct PruningCandidate {
    pub block_id: String,
    pub staleness: f64,
    pub tokens: u32,
    pub reason: String,
}

/// Run content heuristics on a single block.
fn run_heuristics(block: &Block) -> HeuristicResult {
    let mut flags = Vec::new();
    let mut recency_boost = 0.0;
    let content = &block.content;

    // Error/traceback detection
    if content.contains("Error:")
        || content.contains("error:")
        || content.contains("Traceback")
        || content.contains("panic!")
        || content.contains("FAILED")
    {
        flags.push(ContentFlag::ErrorContent);
        recency_boost += 0.3;
    }

    // Correction patterns
    if content.contains("actually,")
        || content.contains("wait,")
        || content.contains("I meant")
        || content.contains("correction:")
        || content.contains("sorry,")
    {
        flags.push(ContentFlag::CorrectionPattern);
        recency_boost += 0.2;
    }

    // File references (Unix and Windows paths)
    if content.contains("src/") || content.contains("./") || content.contains(":\\") {
        flags.push(ContentFlag::FileReference);
    }

    // Code content (common patterns)
    if content.contains("fn ") || content.contains("def ") || content.contains("function ") {
        flags.push(ContentFlag::CodeContent);
    }

    // Size flags
    if block.tokens < 20 {
        flags.push(ContentFlag::VeryShort);
    }
    if block.tokens > 2000 {
        flags.push(ContentFlag::VeryLarge);
        recency_boost -= 0.1; // large blocks slightly demoted
    }

    HeuristicResult {
        block_id: block.id.clone(),
        recency_boost,
        flags,
    }
}

/// Generate a human-readable pruning reason.
fn pruning_reason(block: &Block, staleness: f64) -> String {
    if staleness > 50.0 {
        format!(
            "Very stale (score: {staleness:.1}), {} tokens",
            block.tokens
        )
    } else if block.tokens > 2000 {
        format!(
            "Large block ({} tokens), staleness {staleness:.1}",
            block.tokens
        )
    } else {
        format!("Staleness score: {staleness:.1}, {} tokens", block.tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};
    use std::time::Instant;

    fn make_block(id: &str, role: Role, turn: u32, tokens: u32, content: &str) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: content.to_string(),
            tokens,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Recency),
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
                turn_index: turn,
                tool_name: None,
                file_paths: vec![],
            },
        }
    }

    #[test]
    fn test_heuristics_detect_error_content() {
        let block = make_block("b1", Role::Assistant, 0, 100, "Error: file not found");
        let result = run_heuristics(&block);
        assert!(result.flags.contains(&ContentFlag::ErrorContent));
        assert!(result.recency_boost > 0.0);
    }

    #[test]
    fn test_heuristics_detect_correction() {
        let block = make_block("b1", Role::User, 0, 50, "actually, I meant the other file");
        let result = run_heuristics(&block);
        assert!(result.flags.contains(&ContentFlag::CorrectionPattern));
    }

    #[test]
    fn test_pipeline_assigns_zones() {
        let config = PipelineConfig::default();
        let mut blocks = vec![
            make_block("sys", Role::System, 0, 100, "You are a helpful assistant"),
            make_block("u1", Role::User, 1, 50, "Hello"),
            make_block("a1", Role::Assistant, 2, 100, "Hi there"),
        ];

        let result = classify(&mut blocks, &config);

        assert_eq!(blocks[0].zone, Zone::BuiltIn(BuiltInZone::Primacy));
        assert_eq!(result.alert_level, AlertLevel::Normal); // 250 tokens << 200k
    }

    #[test]
    fn test_pipeline_suggests_pruning_over_budget() {
        let config = PipelineConfig {
            token_limit: 200,
            ..PipelineConfig::default()
        };

        let mut blocks = vec![
            make_block("b0", Role::User, 0, 50, "old block"),
            make_block("b1", Role::User, 1, 50, "slightly newer"),
            make_block("b2", Role::User, 2, 50, "recent"),
            make_block("b3", Role::User, 3, 50, "latest"),
        ];

        let result = classify(&mut blocks, &config);

        // 200 tokens used with 200 limit → 100% utilization → Emergency
        assert!(result.alert_level >= AlertLevel::Warning);
        assert!(
            !result.pruning_candidates.is_empty(),
            "Should suggest pruning candidates"
        );
        // Most stale block should be first candidate
        assert_eq!(result.pruning_candidates[0].block_id, "b0");
    }

    #[test]
    fn test_pinned_blocks_not_suggested_for_pruning() {
        use crate::engine::types::PinPosition;

        let config = PipelineConfig {
            token_limit: 100,
            ..PipelineConfig::default()
        };

        let mut blocks = vec![
            make_block("pinned", Role::User, 0, 60, "important pinned content"),
            make_block("free", Role::User, 1, 60, "normal content"),
        ];
        blocks[0].pinned = Some(PinPosition::Top);

        let result = classify(&mut blocks, &config);

        let pruning_ids: Vec<&str> = result
            .pruning_candidates
            .iter()
            .map(|c| c.block_id.as_str())
            .collect();
        assert!(
            !pruning_ids.contains(&"pinned"),
            "Pinned blocks should not be pruning candidates"
        );
    }

    #[test]
    fn test_pipeline_average_runtime_under_2ms_for_typical_batch() {
        let config = PipelineConfig::default();
        let mut blocks: Vec<Block> = (0..120)
            .map(|i| {
                let role = if i % 11 == 0 {
                    Role::System
                } else if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                };
                let content = if i % 9 == 0 {
                    "Error: failed to open src/main.rs".to_string()
                } else if i % 7 == 0 {
                    "actually, I meant a different approach".to_string()
                } else {
                    format!("normal block content {i}")
                };
                make_block(
                    &format!("b{i}"),
                    role,
                    i as u32,
                    120 + (i % 40) as u32,
                    &content,
                )
            })
            .collect();

        // Warm-up run to avoid one-time initialization skew.
        let _ = classify(&mut blocks, &config);

        let iterations = 300u32;
        let started = Instant::now();
        for _ in 0..iterations {
            let _ = classify(&mut blocks, &config);
        }
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let avg_ms = elapsed_ms / iterations as f64;

        assert!(
            avg_ms < 2.0,
            "Expected average classify runtime <2ms, got {avg_ms:.3}ms over {iterations} iterations"
        );
    }
}
