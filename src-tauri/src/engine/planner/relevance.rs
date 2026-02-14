//! Dependency-based relevance boosting for context blocks.
//!
//! When the current turn touches file X, all blocks that reference file X
//! receive a relevance boost. Boosted blocks resist archival and may shift
//! toward recency.

use std::collections::HashMap;

use crate::engine::block::Block;

/// Compute relevance boost scores for blocks based on current-turn file references.
///
/// Returns a map of block_id → boost score. Only blocks with non-zero boost
/// are included. Higher scores indicate stronger relevance to the current task.
pub fn compute_relevance_boosts(
    blocks: &[Block],
    current_turn_files: &[String],
) -> HashMap<String, f64> {
    if current_turn_files.is_empty() {
        return HashMap::new();
    }

    let mut boosts = HashMap::new();

    for block in blocks {
        if block.metadata.file_paths.is_empty() {
            continue;
        }

        let overlap_count = block
            .metadata
            .file_paths
            .iter()
            .filter(|fp| current_turn_files.contains(fp))
            .count();

        if overlap_count > 0 {
            // Base boost per overlapping file, with diminishing returns
            let boost = (overlap_count as f64) * FILE_OVERLAP_BOOST;
            boosts.insert(block.id.clone(), boost.min(MAX_RELEVANCE_BOOST));
        }
    }

    boosts
}

/// Detect whether a task boundary has occurred.
///
/// A task boundary is detected when >50% of the current turn's files
/// are new (not present in the previous turn's file set).
pub fn detect_task_boundary(current_turn_files: &[String], previous_turn_files: &[String]) -> bool {
    if current_turn_files.is_empty() {
        return false;
    }

    let new_file_count = current_turn_files
        .iter()
        .filter(|f| !previous_turn_files.contains(f))
        .count();

    let new_ratio = new_file_count as f64 / current_turn_files.len() as f64;
    new_ratio > TASK_BOUNDARY_THRESHOLD
}

/// Relevance boost per overlapping file reference.
const FILE_OVERLAP_BOOST: f64 = 0.5;

/// Maximum relevance boost from file overlap.
const MAX_RELEVANCE_BOOST: f64 = 2.0;

/// Fraction of new files needed to signal a task boundary.
const TASK_BOUNDARY_THRESHOLD: f64 = 0.50;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};

    fn make_block(id: &str, file_paths: Vec<&str>) -> Block {
        Block {
            id: id.to_string(),
            role: Role::ToolResult,
            block_type: None,
            content: format!("Content of {id}"),
            tokens: 500,
            timestamp: "2026-02-13T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: format!("Content of {id}"),
                    tokens: 500,
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
                turn_index: 1,
                tool_name: None,
                file_paths: file_paths.into_iter().map(String::from).collect(),
            },
        }
    }

    #[test]
    fn test_boost_block_referencing_current_file() {
        let blocks = vec![
            make_block("b1", vec!["src/auth.rs"]),
            make_block("b2", vec!["src/main.rs"]),
        ];
        let current_files = vec!["src/auth.rs".to_string()];

        let boosts = compute_relevance_boosts(&blocks, &current_files);

        assert!(boosts.contains_key("b1"));
        assert!(!boosts.contains_key("b2"));
        assert!((boosts["b1"] - FILE_OVERLAP_BOOST).abs() < f64::EPSILON);
    }

    #[test]
    fn test_no_boost_when_no_overlap() {
        let blocks = vec![make_block("b1", vec!["src/auth.rs"])];
        let current_files = vec!["src/main.rs".to_string()];

        let boosts = compute_relevance_boosts(&blocks, &current_files);
        assert!(boosts.is_empty());
    }

    #[test]
    fn test_no_boost_when_no_current_files() {
        let blocks = vec![make_block("b1", vec!["src/auth.rs"])];
        let current_files: Vec<String> = vec![];

        let boosts = compute_relevance_boosts(&blocks, &current_files);
        assert!(boosts.is_empty());
    }

    #[test]
    fn test_multiple_file_overlap_increases_boost() {
        let blocks = vec![make_block(
            "b1",
            vec!["src/auth.rs", "src/handler.rs", "src/config.rs"],
        )];
        let current_files = vec!["src/auth.rs".to_string(), "src/handler.rs".to_string()];

        let boosts = compute_relevance_boosts(&blocks, &current_files);

        assert!(boosts["b1"] > FILE_OVERLAP_BOOST);
        assert!((boosts["b1"] - 2.0 * FILE_OVERLAP_BOOST).abs() < f64::EPSILON);
    }

    #[test]
    fn test_boost_capped_at_maximum() {
        let files: Vec<&str> = (0..10)
            .map(|i| match i {
                0 => "a.rs",
                1 => "b.rs",
                2 => "c.rs",
                3 => "d.rs",
                4 => "e.rs",
                5 => "f.rs",
                6 => "g.rs",
                7 => "h.rs",
                8 => "i.rs",
                _ => "j.rs",
            })
            .collect();
        let blocks = vec![make_block("b1", files.clone())];
        let current_files: Vec<String> = files.into_iter().map(String::from).collect();

        let boosts = compute_relevance_boosts(&blocks, &current_files);

        assert!(boosts["b1"] <= MAX_RELEVANCE_BOOST);
    }

    #[test]
    fn test_blocks_without_file_paths_skipped() {
        let blocks = vec![make_block("b1", vec![])];
        let current_files = vec!["src/auth.rs".to_string()];

        let boosts = compute_relevance_boosts(&blocks, &current_files);
        assert!(boosts.is_empty());
    }

    #[test]
    fn test_multiple_blocks_same_file() {
        let blocks = vec![
            make_block("b1", vec!["src/auth.rs"]),
            make_block("b2", vec!["src/auth.rs"]),
            make_block("b3", vec!["src/other.rs"]),
        ];
        let current_files = vec!["src/auth.rs".to_string()];

        let boosts = compute_relevance_boosts(&blocks, &current_files);

        assert_eq!(boosts.len(), 2);
        assert!(boosts.contains_key("b1"));
        assert!(boosts.contains_key("b2"));
        assert!(!boosts.contains_key("b3"));
    }

    // ── Task Boundary Detection ──────────────────────────────

    #[test]
    fn test_task_boundary_all_new_files() {
        let current = vec!["src/new1.rs".to_string(), "src/new2.rs".to_string()];
        let previous = vec!["src/old.rs".to_string()];

        assert!(detect_task_boundary(&current, &previous));
    }

    #[test]
    fn test_no_task_boundary_same_files() {
        let current = vec!["src/auth.rs".to_string(), "src/handler.rs".to_string()];
        let previous = vec!["src/auth.rs".to_string(), "src/handler.rs".to_string()];

        assert!(!detect_task_boundary(&current, &previous));
    }

    #[test]
    fn test_task_boundary_exactly_50_percent_is_not_boundary() {
        // 1 new out of 2 = 50%, threshold is >50%
        let current = vec!["src/auth.rs".to_string(), "src/new.rs".to_string()];
        let previous = vec!["src/auth.rs".to_string()];

        assert!(!detect_task_boundary(&current, &previous));
    }

    #[test]
    fn test_task_boundary_above_50_percent() {
        // 2 new out of 3 = 66.7% > 50%
        let current = vec![
            "src/auth.rs".to_string(),
            "src/new1.rs".to_string(),
            "src/new2.rs".to_string(),
        ];
        let previous = vec!["src/auth.rs".to_string()];

        assert!(detect_task_boundary(&current, &previous));
    }

    #[test]
    fn test_no_task_boundary_empty_current() {
        let current: Vec<String> = vec![];
        let previous = vec!["src/auth.rs".to_string()];

        assert!(!detect_task_boundary(&current, &previous));
    }

    #[test]
    fn test_task_boundary_empty_previous() {
        // All files are new when previous is empty
        let current = vec!["src/auth.rs".to_string()];
        let previous: Vec<String> = vec![];

        assert!(detect_task_boundary(&current, &previous));
    }
}
