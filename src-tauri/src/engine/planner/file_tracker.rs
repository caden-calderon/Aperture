//! File mutation tracking across tool calls.
//!
//! When the proxy sees a file edit (via tool call results), this module
//! detects the mutation and maps it to blocks containing reads of the same
//! file. The model never sees stale file content.

use serde::{Deserialize, Serialize};

use crate::engine::block::Block;
use crate::engine::planner::types::ContextMutation;

// ── File Mutation Types ──────────────────────────────────────

/// A detected file mutation from tool call traffic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMutation {
    /// The file path that was modified.
    pub file_path: String,
    /// The kind of mutation detected.
    pub kind: FileMutationKind,
    /// New content (if available from tool result).
    pub new_content: Option<String>,
}

/// The kind of file mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileMutationKind {
    /// File was written/created.
    Write,
    /// File was edited/patched.
    Edit,
    /// File was deleted.
    Delete,
    /// File was renamed (old_path stored in file_path, new in new_content).
    Rename,
}

// ── Tool Call Parsing ────────────────────────────────────────

/// A parsed tool call with name and arguments for file mutation detection.
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    /// The tool/function name.
    pub name: String,
    /// The tool arguments as a JSON value.
    pub arguments: serde_json::Value,
    /// The tool result content (if available).
    pub result: Option<String>,
}

/// Detect file mutations from a list of tool calls.
///
/// Parses tool call names and arguments to find file write/edit/delete operations.
/// Supports multiple tool name conventions across different LLM tools.
pub fn detect_file_mutations(tool_calls: &[ToolCallInfo]) -> Vec<FileMutation> {
    let mut mutations = Vec::new();

    for call in tool_calls {
        if let Some(mutation) = parse_tool_call(call) {
            mutations.push(mutation);
        }
    }

    mutations
}

/// Find all blocks that reference a given file path.
pub fn find_blocks_for_file<'a>(blocks: &'a [Block], file_path: &str) -> Vec<&'a str> {
    blocks
        .iter()
        .filter(|b| b.metadata.file_paths.iter().any(|fp| fp == file_path))
        .map(|b| b.id.as_str())
        .collect()
}

/// Generate UpdateContent mutations for blocks affected by file mutations.
///
/// For each file mutation with new content, finds all blocks referencing that
/// file and generates an UpdateContent mutation with the new content.
pub fn generate_file_update_mutations(
    file_mutations: &[FileMutation],
    blocks: &[Block],
) -> Vec<ContextMutation> {
    let mut mutations = Vec::new();

    for fm in file_mutations {
        match fm.kind {
            FileMutationKind::Write | FileMutationKind::Edit => {
                if let Some(ref new_content) = fm.new_content {
                    let block_ids = find_blocks_for_file(blocks, &fm.file_path);
                    for block_id in block_ids {
                        mutations.push(ContextMutation::UpdateContent {
                            block_id: block_id.to_string(),
                            new_content: new_content.clone(),
                        });
                    }
                }
            }
            FileMutationKind::Delete => {
                // Mark blocks referencing deleted files for archival
                let block_ids = find_blocks_for_file(blocks, &fm.file_path);
                for block_id in block_ids {
                    mutations.push(ContextMutation::Archive {
                        block_id: block_id.to_string(),
                    });
                }
            }
            FileMutationKind::Rename => {
                // Rename: new_content holds the new path. We can't update content
                // without the actual file contents, so we just note the blocks
                // will need attention. For now, no mutation generated — the next
                // read of the new path will create a fresh block.
            }
        }
    }

    mutations
}

// ── Internal Parsing ─────────────────────────────────────────

/// Known tool name patterns for file mutations.
/// Maps (tool_name_pattern, mutation_kind).
const WRITE_TOOL_NAMES: &[&str] = &[
    "write_file",
    "write_to_file",
    "create_file",
    "Write",
    "write",
];

const EDIT_TOOL_NAMES: &[&str] = &[
    "edit_file",
    "apply_diff",
    "Edit",
    "edit",
    "str_replace_editor",
    "replace_in_file",
    "patch_file",
    "insert_code_block",
];

const DELETE_TOOL_NAMES: &[&str] = &["delete_file", "remove_file"];

const RENAME_TOOL_NAMES: &[&str] = &["rename_file", "move_file"];

/// Parse a single tool call into a file mutation (if it is one).
fn parse_tool_call(call: &ToolCallInfo) -> Option<FileMutation> {
    let name = call.name.as_str();

    let kind = if WRITE_TOOL_NAMES.contains(&name) {
        FileMutationKind::Write
    } else if EDIT_TOOL_NAMES.contains(&name) {
        FileMutationKind::Edit
    } else if DELETE_TOOL_NAMES.contains(&name) {
        FileMutationKind::Delete
    } else if RENAME_TOOL_NAMES.contains(&name) {
        FileMutationKind::Rename
    } else {
        return None;
    };

    let file_path = extract_file_path(&call.arguments)?;

    let new_content = match kind {
        FileMutationKind::Write => {
            // For writes, content is in the arguments
            extract_content(&call.arguments).or(call.result.clone())
        }
        FileMutationKind::Edit => {
            // For edits, the result typically contains updated content
            call.result.clone()
        }
        FileMutationKind::Delete => None,
        FileMutationKind::Rename => {
            // For renames, extract the new path
            extract_new_path(&call.arguments)
        }
    };

    Some(FileMutation {
        file_path,
        kind,
        new_content,
    })
}

/// Extract file path from tool call arguments.
/// Tries common argument names: file_path, path, filename, file.
fn extract_file_path(args: &serde_json::Value) -> Option<String> {
    let obj = args.as_object()?;

    for key in &["file_path", "path", "filename", "file", "filePath"] {
        if let Some(val) = obj.get(*key) {
            if let Some(s) = val.as_str() {
                return Some(s.to_string());
            }
        }
    }

    // str_replace_editor uses "command" + "path" pattern
    if let Some(val) = obj.get("file_name") {
        if let Some(s) = val.as_str() {
            return Some(s.to_string());
        }
    }

    None
}

/// Extract content from tool call arguments.
fn extract_content(args: &serde_json::Value) -> Option<String> {
    let obj = args.as_object()?;

    for key in &["content", "new_content", "text", "file_text"] {
        if let Some(val) = obj.get(*key) {
            if let Some(s) = val.as_str() {
                return Some(s.to_string());
            }
        }
    }

    None
}

/// Extract new path from rename arguments.
fn extract_new_path(args: &serde_json::Value) -> Option<String> {
    let obj = args.as_object()?;

    for key in &["new_path", "destination", "new_name", "to"] {
        if let Some(val) = obj.get(*key) {
            if let Some(s) = val.as_str() {
                return Some(s.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};
    use serde_json::json;

    fn make_block(id: &str, file_paths: Vec<&str>) -> Block {
        Block {
            id: id.to_string(),
            role: Role::ToolResult,
            block_type: None,
            content: format!("Content of file read for {id}"),
            tokens: 500,
            timestamp: "2026-02-13T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Middle),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: format!("Content of file read for {id}"),
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

    fn make_tool_call(name: &str, args: serde_json::Value, result: Option<&str>) -> ToolCallInfo {
        ToolCallInfo {
            name: name.to_string(),
            arguments: args,
            result: result.map(String::from),
        }
    }

    // ── Tool Call Detection ──────────────────────────────────

    #[test]
    fn test_detect_write_file() {
        let calls = vec![make_tool_call(
            "write_file",
            json!({"file_path": "src/auth.rs", "content": "fn new_auth() {}"}),
            None,
        )];

        let mutations = detect_file_mutations(&calls);
        assert_eq!(mutations.len(), 1);
        assert_eq!(mutations[0].file_path, "src/auth.rs");
        assert_eq!(mutations[0].kind, FileMutationKind::Write);
        assert_eq!(
            mutations[0].new_content.as_deref(),
            Some("fn new_auth() {}")
        );
    }

    #[test]
    fn test_detect_write_tool_uppercase() {
        let calls = vec![make_tool_call(
            "Write",
            json!({"file_path": "src/main.rs", "content": "fn main() {}"}),
            None,
        )];

        let mutations = detect_file_mutations(&calls);
        assert_eq!(mutations.len(), 1);
        assert_eq!(mutations[0].kind, FileMutationKind::Write);
    }

    #[test]
    fn test_detect_edit_file() {
        let calls = vec![make_tool_call(
            "edit_file",
            json!({"path": "src/auth.rs"}),
            Some("fn updated_auth() {}"),
        )];

        let mutations = detect_file_mutations(&calls);
        assert_eq!(mutations.len(), 1);
        assert_eq!(mutations[0].kind, FileMutationKind::Edit);
        assert_eq!(
            mutations[0].new_content.as_deref(),
            Some("fn updated_auth() {}")
        );
    }

    #[test]
    fn test_detect_str_replace_editor() {
        let calls = vec![make_tool_call(
            "str_replace_editor",
            json!({"path": "src/handler.rs", "command": "str_replace"}),
            Some("fn handle() { updated }"),
        )];

        let mutations = detect_file_mutations(&calls);
        assert_eq!(mutations.len(), 1);
        assert_eq!(mutations[0].file_path, "src/handler.rs");
        assert_eq!(mutations[0].kind, FileMutationKind::Edit);
    }

    #[test]
    fn test_detect_delete_file() {
        let calls = vec![make_tool_call(
            "delete_file",
            json!({"file_path": "src/old.rs"}),
            None,
        )];

        let mutations = detect_file_mutations(&calls);
        assert_eq!(mutations.len(), 1);
        assert_eq!(mutations[0].kind, FileMutationKind::Delete);
        assert!(mutations[0].new_content.is_none());
    }

    #[test]
    fn test_detect_rename_file() {
        let calls = vec![make_tool_call(
            "rename_file",
            json!({"file_path": "src/old.rs", "new_path": "src/new.rs"}),
            None,
        )];

        let mutations = detect_file_mutations(&calls);
        assert_eq!(mutations.len(), 1);
        assert_eq!(mutations[0].kind, FileMutationKind::Rename);
        assert_eq!(mutations[0].file_path, "src/old.rs");
        assert_eq!(mutations[0].new_content.as_deref(), Some("src/new.rs"));
    }

    #[test]
    fn test_ignore_non_file_tool_calls() {
        let calls = vec![
            make_tool_call("read_file", json!({"path": "src/main.rs"}), None),
            make_tool_call("search", json!({"query": "auth"}), None),
            make_tool_call("run_tests", json!({}), None),
        ];

        let mutations = detect_file_mutations(&calls);
        assert!(mutations.is_empty());
    }

    #[test]
    fn test_missing_file_path_returns_none() {
        let calls = vec![make_tool_call(
            "write_file",
            json!({"unknown_key": "value"}),
            None,
        )];

        let mutations = detect_file_mutations(&calls);
        assert!(mutations.is_empty());
    }

    #[test]
    fn test_multiple_mutations_in_one_batch() {
        let calls = vec![
            make_tool_call(
                "write_file",
                json!({"file_path": "src/a.rs", "content": "new a"}),
                None,
            ),
            make_tool_call("edit_file", json!({"path": "src/b.rs"}), Some("new b")),
            make_tool_call("delete_file", json!({"file_path": "src/c.rs"}), None),
        ];

        let mutations = detect_file_mutations(&calls);
        assert_eq!(mutations.len(), 3);
    }

    // ── Block Finding ────────────────────────────────────────

    #[test]
    fn test_find_blocks_for_file() {
        let blocks = vec![
            make_block("b1", vec!["src/auth.rs"]),
            make_block("b2", vec!["src/auth.rs", "src/handler.rs"]),
            make_block("b3", vec!["src/main.rs"]),
        ];

        let found = find_blocks_for_file(&blocks, "src/auth.rs");
        assert_eq!(found, vec!["b1", "b2"]);
    }

    #[test]
    fn test_find_blocks_no_match() {
        let blocks = vec![make_block("b1", vec!["src/main.rs"])];

        let found = find_blocks_for_file(&blocks, "src/other.rs");
        assert!(found.is_empty());
    }

    // ── Mutation Generation ──────────────────────────────────

    #[test]
    fn test_generate_update_mutations_for_write() {
        let blocks = vec![
            make_block("b1", vec!["src/auth.rs"]),
            make_block("b2", vec!["src/main.rs"]),
        ];
        let file_mutations = vec![FileMutation {
            file_path: "src/auth.rs".to_string(),
            kind: FileMutationKind::Write,
            new_content: Some("fn new_auth() {}".to_string()),
        }];

        let mutations = generate_file_update_mutations(&file_mutations, &blocks);
        assert_eq!(mutations.len(), 1);
        assert!(matches!(
            &mutations[0],
            ContextMutation::UpdateContent { block_id, new_content }
                if block_id == "b1" && new_content == "fn new_auth() {}"
        ));
    }

    #[test]
    fn test_generate_archive_mutation_for_delete() {
        let blocks = vec![make_block("b1", vec!["src/old.rs"])];
        let file_mutations = vec![FileMutation {
            file_path: "src/old.rs".to_string(),
            kind: FileMutationKind::Delete,
            new_content: None,
        }];

        let mutations = generate_file_update_mutations(&file_mutations, &blocks);
        assert_eq!(mutations.len(), 1);
        assert!(matches!(
            &mutations[0],
            ContextMutation::Archive { block_id } if block_id == "b1"
        ));
    }

    #[test]
    fn test_multiple_blocks_same_file_all_updated() {
        let blocks = vec![
            make_block("b1", vec!["src/auth.rs"]),
            make_block("b2", vec!["src/auth.rs"]),
        ];
        let file_mutations = vec![FileMutation {
            file_path: "src/auth.rs".to_string(),
            kind: FileMutationKind::Edit,
            new_content: Some("updated content".to_string()),
        }];

        let mutations = generate_file_update_mutations(&file_mutations, &blocks);
        assert_eq!(mutations.len(), 2);
    }

    #[test]
    fn test_no_mutation_without_new_content() {
        let blocks = vec![make_block("b1", vec!["src/auth.rs"])];
        let file_mutations = vec![FileMutation {
            file_path: "src/auth.rs".to_string(),
            kind: FileMutationKind::Edit,
            new_content: None, // No content available
        }];

        let mutations = generate_file_update_mutations(&file_mutations, &blocks);
        assert!(mutations.is_empty());
    }

    #[test]
    fn test_rename_generates_no_mutations() {
        let blocks = vec![make_block("b1", vec!["src/old.rs"])];
        let file_mutations = vec![FileMutation {
            file_path: "src/old.rs".to_string(),
            kind: FileMutationKind::Rename,
            new_content: Some("src/new.rs".to_string()),
        }];

        let mutations = generate_file_update_mutations(&file_mutations, &blocks);
        assert!(mutations.is_empty());
    }

    // ── Path Extraction ──────────────────────────────────────

    #[test]
    fn test_extract_file_path_various_keys() {
        assert_eq!(
            extract_file_path(&json!({"file_path": "a.rs"})),
            Some("a.rs".to_string())
        );
        assert_eq!(
            extract_file_path(&json!({"path": "b.rs"})),
            Some("b.rs".to_string())
        );
        assert_eq!(
            extract_file_path(&json!({"filename": "c.rs"})),
            Some("c.rs".to_string())
        );
        assert_eq!(
            extract_file_path(&json!({"file": "d.rs"})),
            Some("d.rs".to_string())
        );
        assert_eq!(
            extract_file_path(&json!({"filePath": "e.rs"})),
            Some("e.rs".to_string())
        );
    }

    #[test]
    fn test_extract_file_path_missing() {
        assert_eq!(extract_file_path(&json!({"other": "value"})), None);
        assert_eq!(extract_file_path(&json!(42)), None);
    }
}
