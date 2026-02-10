//! Deterministic dependency tracking between blocks.
//!
//! Tracks three types of relationships:
//! 1. File reference chains — block reads a file, later blocks reference it
//! 2. Conversation flow — user → assistant → tool chains
//! 3. Tool chains — tool_use → tool_result pairs
//!
//! Phase 7 extends this with semantic dependencies from the cleaner sidecar.

use std::collections::HashSet;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use super::block::Block;
use super::types::Role;

/// A directed dependency edge between two blocks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyKind {
    /// Block references a file path mentioned in another block.
    FileReference { path: String },
    /// Conversation flow (e.g., user message → assistant response).
    ConversationFlow,
    /// Tool use → tool result link.
    ToolChain { tool_name: String },
}

/// A dependency edge from one block to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// Block that depends on another.
    pub from_id: String,
    /// Block that is depended upon.
    pub to_id: String,
    /// Type of dependency.
    pub kind: DependencyKind,
}

/// Dependency graph tracking all block relationships.
pub struct DependencyGraph {
    /// Edges keyed by "from" block ID.
    forward: DashMap<String, Vec<DependencyEdge>>,
    /// Reverse index: "to" block ID → edges pointing to it.
    reverse: DashMap<String, Vec<DependencyEdge>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            forward: DashMap::new(),
            reverse: DashMap::new(),
        }
    }

    /// Add a dependency edge.
    pub fn add_edge(&self, edge: DependencyEdge) {
        self.reverse
            .entry(edge.to_id.clone())
            .or_default()
            .push(edge.clone());
        self.forward
            .entry(edge.from_id.clone())
            .or_default()
            .push(edge);
    }

    /// Get all blocks that a given block depends on.
    pub fn dependencies_of(&self, block_id: &str) -> Vec<DependencyEdge> {
        self.forward
            .get(block_id)
            .map(|r| r.value().clone())
            .unwrap_or_default()
    }

    /// Get all blocks that depend on a given block.
    pub fn dependents_of(&self, block_id: &str) -> Vec<DependencyEdge> {
        self.reverse
            .get(block_id)
            .map(|r| r.value().clone())
            .unwrap_or_default()
    }

    /// Remove all edges involving a block (forward and reverse).
    pub fn remove_block(&self, block_id: &str) {
        // Remove forward edges
        if let Some((_, edges)) = self.forward.remove(block_id) {
            for edge in &edges {
                self.reverse.alter(&edge.to_id, |_, mut v| {
                    v.retain(|e| e.from_id != block_id);
                    v
                });
            }
        }
        // Remove reverse edges
        if let Some((_, edges)) = self.reverse.remove(block_id) {
            for edge in &edges {
                self.forward.alter(&edge.from_id, |_, mut v| {
                    v.retain(|e| e.to_id != block_id);
                    v
                });
            }
        }
    }

    /// Find orphaned blocks — blocks whose file dependencies are gone.
    pub fn find_orphans(&self, existing_block_ids: &HashSet<String>) -> Vec<String> {
        let mut orphans = Vec::new();

        for entry in self.forward.iter() {
            let block_id = entry.key();
            let edges = entry.value();

            for edge in edges {
                if !existing_block_ids.contains(&edge.to_id) {
                    orphans.push(block_id.clone());
                    break;
                }
            }
        }

        orphans
    }

    /// Total number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.forward.iter().map(|r| r.value().len()).sum()
    }

    /// Clear all edges.
    pub fn clear(&self) {
        self.forward.clear();
        self.reverse.clear();
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Build dependency edges from a set of blocks.
///
/// Detects:
/// 1. Conversation flow (sequential turn pairs)
/// 2. Tool chains (tool_use → tool_result by tool name)
/// 3. File references (blocks mentioning same file paths)
pub fn build_dependencies(blocks: &[Block]) -> Vec<DependencyEdge> {
    let mut edges = Vec::new();

    // Sort by turn index for sequential analysis
    let mut sorted: Vec<&Block> = blocks.iter().collect();
    sorted.sort_by_key(|b| b.metadata.turn_index);

    // 1. Conversation flow: consecutive turns form a chain
    for window in sorted.windows(2) {
        let prev = window[0];
        let curr = window[1];

        // User→Assistant or Assistant→ToolUse or ToolResult→User flow
        if is_conversational_pair(prev.role, curr.role) {
            edges.push(DependencyEdge {
                from_id: curr.id.clone(),
                to_id: prev.id.clone(),
                kind: DependencyKind::ConversationFlow,
            });
        }
    }

    // 2. Tool chains: match tool_use blocks with their tool_result
    let tool_uses: Vec<&Block> = blocks.iter().filter(|b| b.role == Role::ToolUse).collect();
    let tool_results: Vec<&Block> = blocks
        .iter()
        .filter(|b| b.role == Role::ToolResult)
        .collect();

    for tu in &tool_uses {
        if let Some(tool_name) = &tu.metadata.tool_name {
            // Find matching tool_result by tool name and turn proximity
            if let Some(tr) = tool_results.iter().find(|tr| {
                tr.metadata.tool_name.as_deref() == Some(tool_name)
                    && tr.metadata.turn_index > tu.metadata.turn_index
            }) {
                edges.push(DependencyEdge {
                    from_id: tr.id.clone(),
                    to_id: tu.id.clone(),
                    kind: DependencyKind::ToolChain {
                        tool_name: tool_name.clone(),
                    },
                });
            }
        }
    }

    // 3. File references: blocks mentioning the same file path
    let blocks_with_files: Vec<(&Block, &[String])> = blocks
        .iter()
        .filter(|b| !b.metadata.file_paths.is_empty())
        .map(|b| (b, b.metadata.file_paths.as_slice()))
        .collect();

    for i in 0..blocks_with_files.len() {
        for j in (i + 1)..blocks_with_files.len() {
            let (b1, paths1) = &blocks_with_files[i];
            let (b2, paths2) = &blocks_with_files[j];

            for path in *paths1 {
                if paths2.contains(path) {
                    // Later block depends on earlier one (file context)
                    let (from, to) = if b1.metadata.turn_index < b2.metadata.turn_index {
                        (&b2.id, &b1.id)
                    } else {
                        (&b1.id, &b2.id)
                    };
                    edges.push(DependencyEdge {
                        from_id: from.clone(),
                        to_id: to.clone(),
                        kind: DependencyKind::FileReference { path: path.clone() },
                    });
                    break; // one edge per block pair
                }
            }
        }
    }

    edges
}

/// Check if two consecutive roles form a natural conversation pair.
fn is_conversational_pair(prev: Role, curr: Role) -> bool {
    matches!(
        (prev, curr),
        (Role::User, Role::Assistant)
            | (Role::Assistant, Role::ToolUse)
            | (Role::ToolUse, Role::ToolResult)
            | (Role::ToolResult, Role::Assistant)
            | (Role::ToolResult, Role::User)
            | (Role::Assistant, Role::User)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Zone};

    fn make_block(id: &str, role: Role, turn: u32, tool: Option<&str>, files: Vec<&str>) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: "test".to_string(),
            tokens: 10,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Recency),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: "test".to_string(),
                    tokens: 10,
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
                tool_name: tool.map(String::from),
                file_paths: files.into_iter().map(String::from).collect(),
            },
        }
    }

    #[test]
    fn test_conversation_flow_edges() {
        let blocks = vec![
            make_block("u1", Role::User, 0, None, vec![]),
            make_block("a1", Role::Assistant, 1, None, vec![]),
        ];

        let edges = build_dependencies(&blocks);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_id, "a1");
        assert_eq!(edges[0].to_id, "u1");
        assert_eq!(edges[0].kind, DependencyKind::ConversationFlow);
    }

    #[test]
    fn test_tool_chain_edges() {
        let blocks = vec![
            make_block("tu1", Role::ToolUse, 1, Some("read_file"), vec![]),
            make_block("tr1", Role::ToolResult, 2, Some("read_file"), vec![]),
        ];

        let edges = build_dependencies(&blocks);
        let tool_edges: Vec<_> = edges
            .iter()
            .filter(|e| matches!(e.kind, DependencyKind::ToolChain { .. }))
            .collect();

        assert_eq!(tool_edges.len(), 1);
        assert_eq!(tool_edges[0].from_id, "tr1");
        assert_eq!(tool_edges[0].to_id, "tu1");
    }

    #[test]
    fn test_file_reference_edges() {
        let blocks = vec![
            make_block("b1", Role::ToolResult, 0, None, vec!["src/main.rs"]),
            make_block("b2", Role::Assistant, 1, None, vec!["src/main.rs"]),
        ];

        let edges = build_dependencies(&blocks);
        let file_edges: Vec<_> = edges
            .iter()
            .filter(|e| matches!(e.kind, DependencyKind::FileReference { .. }))
            .collect();

        assert_eq!(file_edges.len(), 1);
        assert_eq!(file_edges[0].from_id, "b2"); // later block depends on earlier
        assert_eq!(file_edges[0].to_id, "b1");
    }

    #[test]
    fn test_graph_operations() {
        let graph = DependencyGraph::new();

        graph.add_edge(DependencyEdge {
            from_id: "b2".into(),
            to_id: "b1".into(),
            kind: DependencyKind::ConversationFlow,
        });

        assert_eq!(graph.dependencies_of("b2").len(), 1);
        assert_eq!(graph.dependents_of("b1").len(), 1);
        assert_eq!(graph.edge_count(), 1);

        graph.remove_block("b2");
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.dependents_of("b1").is_empty());
    }

    #[test]
    fn test_orphan_detection() {
        let graph = DependencyGraph::new();
        graph.add_edge(DependencyEdge {
            from_id: "b2".into(),
            to_id: "b1".into(),
            kind: DependencyKind::FileReference {
                path: "file.rs".into(),
            },
        });

        let mut existing = HashSet::new();
        existing.insert("b2".to_string());
        // b1 is missing → b2 is orphaned

        let orphans = graph.find_orphans(&existing);
        assert_eq!(orphans, vec!["b2"]);
    }
}
