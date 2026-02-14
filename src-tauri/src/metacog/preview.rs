//! Smart preview extraction for context blocks.
//!
//! Extracts identifying elements from block content: function/class names,
//! file paths, import statements, code signatures. Not a summary — a table
//! of contents giving enough signal for the model to decide "do I need this?"

use std::collections::HashSet;

use lazy_static::lazy_static;
use regex::Regex;

use crate::engine::block::Block;
use crate::engine::types::Role;

/// Maximum preview length in characters.
const MAX_PREVIEW_CHARS: usize = 300;

/// Maximum number of extracted identifiers to include.
const MAX_IDENTIFIERS: usize = 8;

/// Number of lines to include from beginning/end for non-code blocks.
const CONTEXT_LINES: usize = 3;

lazy_static! {
    // Function/method definitions
    static ref RE_RUST_FN: Regex = Regex::new(r"(?m)^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)").expect("regex");
    static ref RE_PYTHON_DEF: Regex = Regex::new(r"(?m)^\s*(?:async\s+)?def\s+(\w+)").expect("regex");
    static ref RE_JS_FUNCTION: Regex = Regex::new(r"(?m)(?:function\s+(\w+)|(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:\([^)]*\)\s*=>|\w+\s*=>))").expect("regex");

    // Class/struct/type definitions
    static ref RE_CLASS: Regex = Regex::new(r"(?m)^\s*(?:pub\s+)?(?:class|struct|enum|trait|interface|type)\s+(\w+)").expect("regex");

    // Import/use statements
    static ref RE_IMPORT: Regex = Regex::new(r"(?m)^\s*(?:use|import|from|require)\s+(.+?)(?:;|\n)").expect("regex");

    // File paths (in content, tool results, etc.)
    static ref RE_FILE_PATH: Regex = Regex::new(r#"(?:^|\s|[`"'])([/.][\w./\-]+\.\w{1,8})(?:\s|$|[`"':])"#).expect("regex");
}

/// A smart-extracted preview of a block's content.
#[derive(Debug, Clone)]
pub struct BlockPreview {
    /// The block ID.
    pub block_id: String,
    /// Short role label.
    pub role: String,
    /// Zone name.
    pub zone: String,
    /// Token count.
    pub tokens: u32,
    /// Extracted identifiers (function names, class names, file paths).
    pub identifiers: Vec<String>,
    /// First few lines of content.
    pub head: String,
    /// Whether the block is compressed.
    pub is_compressed: bool,
    /// Whether the block is pinned.
    pub is_pinned: bool,
}

impl BlockPreview {
    /// Format as a concise single-line summary for manifest use.
    pub fn to_summary_line(&self) -> String {
        let flags = {
            let mut f = Vec::new();
            if self.is_compressed {
                f.push("compressed");
            }
            if self.is_pinned {
                f.push("pinned");
            }
            if f.is_empty() {
                String::new()
            } else {
                format!(" [{}]", f.join(", "))
            }
        };

        let ids = if self.identifiers.is_empty() {
            String::new()
        } else {
            format!(" — {}", self.identifiers.join(", "))
        };

        format!(
            "#{} {} ({} tok, {}){flags}{ids}",
            self.block_id, self.role, self.tokens, self.zone
        )
    }
}

/// Extract a smart preview from a block.
pub fn extract_preview(block: &Block) -> BlockPreview {
    let zone = format!("{:?}", block.zone).to_lowercase();
    let role = format!("{:?}", block.role).to_lowercase();

    let identifiers = extract_identifiers(&block.content, &block.role);
    let head = extract_head(&block.content, &block.role);

    BlockPreview {
        block_id: block.id.clone(),
        role,
        zone,
        tokens: block.tokens,
        identifiers,
        head,
        is_compressed: block.compression_level != crate::engine::types::CompressionLevel::Original,
        is_pinned: block.pinned.is_some(),
    }
}

/// Extract identifying elements from block content.
fn extract_identifiers(content: &str, role: &Role) -> Vec<String> {
    let mut identifiers = Vec::new();
    let mut seen = HashSet::new();

    // Extract function/method names
    for cap in RE_RUST_FN.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            let n = name.as_str().to_string();
            if seen.insert(n.clone()) {
                identifiers.push(format!("fn {n}()"));
            }
        }
    }

    for cap in RE_PYTHON_DEF.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            let n = name.as_str().to_string();
            if seen.insert(n.clone()) {
                identifiers.push(format!("def {n}()"));
            }
        }
    }

    for cap in RE_JS_FUNCTION.captures_iter(content) {
        let name = cap
            .get(1)
            .or_else(|| cap.get(2))
            .map(|m| m.as_str().to_string());
        if let Some(n) = name {
            if seen.insert(n.clone()) {
                identifiers.push(format!("{n}()"));
            }
        }
    }

    // Extract class/struct/type names
    for cap in RE_CLASS.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            let n = name.as_str().to_string();
            if seen.insert(n.clone()) {
                identifiers.push(n);
            }
        }
    }

    // Extract file paths (especially useful for tool_use/tool_result blocks)
    if matches!(role, Role::ToolUse | Role::ToolResult) {
        for cap in RE_FILE_PATH.captures_iter(content) {
            if let Some(path) = cap.get(1) {
                let p = path.as_str().to_string();
                if seen.insert(p.clone()) {
                    identifiers.push(p);
                }
            }
        }
    }

    // Also check block metadata file_paths (not available here directly,
    // but the caller can augment the preview)

    identifiers.truncate(MAX_IDENTIFIERS);
    identifiers
}

/// Extract the head (first few lines) of content.
fn extract_head(content: &str, role: &Role) -> String {
    let lines: Vec<&str> = content.lines().collect();

    if lines.is_empty() {
        return String::new();
    }

    // For code-heavy blocks, use more lines
    let n = match role {
        Role::ToolResult | Role::ToolUse => CONTEXT_LINES + 2,
        _ => CONTEXT_LINES,
    };

    let head_lines: Vec<&str> = lines.iter().take(n).copied().collect();
    let mut head = head_lines.join("\n");

    if head.len() > MAX_PREVIEW_CHARS {
        head.truncate(MAX_PREVIEW_CHARS);
        head.push_str("...");
    } else if lines.len() > n {
        head.push_str("\n...");
    }

    head
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::block::{BlockMetadata, CompressionVersion, CompressionVersions};
    use crate::engine::types::{BuiltInZone, CompressionLevel, Zone};

    fn make_block(id: &str, role: Role, content: &str) -> Block {
        Block {
            id: id.to_string(),
            role,
            block_type: None,
            content: content.to_string(),
            tokens: (content.len() as u32) / 4,
            timestamp: "2026-02-13T00:00:00Z".to_string(),
            zone: Zone::BuiltIn(BuiltInZone::Recency),
            pinned: None,
            compression_level: CompressionLevel::Original,
            compressed_versions: CompressionVersions {
                original: CompressionVersion {
                    content: content.to_string(),
                    tokens: (content.len() as u32) / 4,
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

    #[test]
    fn test_extract_rust_functions() {
        let content = r#"
pub fn calculate_tokens(content: &str) -> u32 {
    content.len() as u32 / 4
}

pub async fn handle_request(req: Request) -> Response {
    todo!()
}

fn helper() {}
"#;
        let ids = extract_identifiers(content, &Role::Assistant);
        assert!(ids.iter().any(|i| i.contains("calculate_tokens")));
        assert!(ids.iter().any(|i| i.contains("handle_request")));
        assert!(ids.iter().any(|i| i.contains("helper")));
    }

    #[test]
    fn test_extract_python_functions() {
        let content = r#"
def process_data(items):
    return [x for x in items]

async def fetch_url(url):
    pass

class DataProcessor:
    pass
"#;
        let ids = extract_identifiers(content, &Role::Assistant);
        assert!(ids.iter().any(|i| i.contains("process_data")));
        assert!(ids.iter().any(|i| i.contains("fetch_url")));
        assert!(ids.iter().any(|i| i.contains("DataProcessor")));
    }

    #[test]
    fn test_extract_classes_and_structs() {
        let content = r#"
pub struct ContextEngine {
    store: BlockStore,
}

pub enum Zone {
    Primacy,
    Middle,
}

pub trait Runtime {
    fn plan(&self);
}
"#;
        let ids = extract_identifiers(content, &Role::Assistant);
        assert!(ids.iter().any(|i| i == "ContextEngine"));
        assert!(ids.iter().any(|i| i == "Zone"));
        assert!(ids.iter().any(|i| i == "Runtime"));
    }

    #[test]
    fn test_extract_file_paths_in_tool_result() {
        let content = r#"
Read file: `/src/engine/mod.rs`
Content: ...
Also references ./lib/utils/syntax.ts in the output.
"#;
        let ids = extract_identifiers(content, &Role::ToolResult);
        assert!(ids
            .iter()
            .any(|i| i.contains("mod.rs") || i.contains("syntax.ts")));
    }

    #[test]
    fn test_extract_preview_basic() {
        let block = make_block(
            "42",
            Role::Assistant,
            "This is a simple response.\nLine 2.\nLine 3.\nLine 4.",
        );
        let preview = extract_preview(&block);
        assert_eq!(preview.block_id, "42");
        assert_eq!(preview.role, "assistant");
        assert!(!preview.is_compressed);
        assert!(!preview.is_pinned);
    }

    #[test]
    fn test_preview_summary_line() {
        let mut block = make_block("7", Role::System, "System prompt content");
        block.zone = Zone::BuiltIn(BuiltInZone::Primacy);
        block.tokens = 150;

        let preview = extract_preview(&block);
        let line = preview.to_summary_line();
        assert!(line.contains("#7"));
        assert!(line.contains("system"));
        assert!(line.contains("150 tok"));
    }

    #[test]
    fn test_head_extraction_short_content() {
        let head = extract_head("Short content", &Role::User);
        assert_eq!(head, "Short content");
    }

    #[test]
    fn test_head_extraction_long_content() {
        let lines: Vec<String> = (0..20).map(|i| format!("Line {i}")).collect();
        let content = lines.join("\n");
        let head = extract_head(&content, &Role::User);
        assert!(head.contains("Line 0"));
        assert!(head.contains("Line 2"));
        assert!(head.ends_with("..."));
    }

    #[test]
    fn test_max_identifiers_cap() {
        // Generate content with many functions
        let funcs: Vec<String> = (0..20).map(|i| format!("pub fn func_{i}() {{}}")).collect();
        let content = funcs.join("\n");
        let ids = extract_identifiers(&content, &Role::Assistant);
        assert!(ids.len() <= MAX_IDENTIFIERS);
    }
}
