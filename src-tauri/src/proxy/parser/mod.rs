//! Request/response parsing for Anthropic and OpenAI API formats.
//!
//! Converts raw API JSON into universal `Block` structs that the context engine
//! and UI can work with. Handles:
//! - Anthropic Messages API (`/v1/messages`)
//! - OpenAI Chat Completions (`/v1/chat/completions`)
//! - OpenAI Responses API (`/v1/responses`)

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
use crate::engine::types::{CompressionLevel, Role, Zone};

// ============================================================================
// Provider detection
// ============================================================================

/// Detected API provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    OpenAI,
}

impl Provider {
    /// Detect provider from request path.
    ///
    /// Matches both `/v1/` prefixed and bare paths (e.g. Codex hits `/responses`
    /// without the `/v1/` prefix).
    pub fn from_path(path: &str) -> Option<Self> {
        if is_messages_path(path) {
            Some(Provider::Anthropic)
        } else if is_chat_completions_path(path) || is_responses_path(path) {
            Some(Provider::OpenAI)
        } else {
            None
        }
    }
}

/// Check if path targets the Anthropic Messages API.
pub(crate) fn is_messages_path(path: &str) -> bool {
    path == "/v1/messages"
        || path.starts_with("/v1/messages?")
        || path.starts_with("/v1/messages/")
        || path == "/messages"
        || path.starts_with("/messages?")
        || path.starts_with("/messages/")
}

/// Check if path targets OpenAI Chat Completions.
pub(crate) fn is_chat_completions_path(path: &str) -> bool {
    path == "/v1/chat/completions"
        || path.starts_with("/v1/chat/completions?")
        || path.starts_with("/v1/chat/completions/")
        || path == "/chat/completions"
        || path.starts_with("/chat/completions?")
        || path.starts_with("/chat/completions/")
}

/// Check if path targets OpenAI Responses API.
pub(crate) fn is_responses_path(path: &str) -> bool {
    path == "/v1/responses"
        || path.starts_with("/v1/responses?")
        || path.starts_with("/v1/responses/")
        || path == "/responses"
        || path.starts_with("/responses?")
        || path.starts_with("/responses/")
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Anthropic => write!(f, "anthropic"),
            Provider::OpenAI => write!(f, "openai"),
        }
    }
}

// ============================================================================
// Parsed output types
// ============================================================================

/// Token usage extracted from an API response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Result of parsing a request body.
#[derive(Debug, Clone)]
pub struct ParsedRequest {
    pub provider: Provider,
    pub model: String,
    pub blocks: Vec<Block>,
    /// Best-effort stable thread identity for session isolation.
    ///
    /// When upstream protocols do not provide an explicit thread/session ID,
    /// this is derived from stable early-turn conversation anchors.
    pub thread_identity: Option<String>,
    /// System prompt extracted from request (Anthropic top-level `system` field).
    pub system_prompt: Option<String>,
    /// Whether the request asks for streaming (`"stream": true`).
    pub stream: bool,
    /// Estimated token overhead from tool definitions, system instructions, and
    /// other non-message content that the LLM counts toward context but Aperture
    /// doesn't track as blocks (e.g. the `tools` array in the request JSON).
    /// Approximated as bytes/4 of the serialized tools array.
    pub overhead_tokens: u32,
}

/// Result of parsing a response body.
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    pub provider: Provider,
    pub blocks: Vec<Block>,
    pub usage: Option<TokenUsage>,
    pub model: Option<String>,
}

mod anthropic;
mod identity;
mod openai;
mod overhead;

pub use anthropic::{parse_anthropic_request, parse_anthropic_response};
pub use openai::{
    parse_openai_chat_request, parse_openai_chat_response, parse_openai_responses_request,
    parse_openai_responses_response,
};
// ============================================================================
// Block construction helper
// ============================================================================

fn now_iso8601() -> String {
    crate::util::iso_now()
}

/// Estimate token count from text content.
///
/// Uses a simple heuristic (chars / 4) as a fast approximation.
/// tiktoken-rs integration is available for precise counts in Phase 2.
fn estimate_tokens(content: &str) -> u32 {
    // Rough approximation: ~4 chars per token for English text
    (content.len() as f64 / 4.0).ceil() as u32
}

fn stable_block_id(role: Role, provider: &str, content_fp: &str, block_key: &str) -> String {
    let seed = format!("{provider}|{role:?}|{content_fp}|{block_key}");
    let hash_with_salt = |salt: u64| -> u64 {
        let mut hasher = DefaultHasher::new();
        salt.hash(&mut hasher);
        seed.hash(&mut hasher);
        hasher.finish()
    };
    let hi = hash_with_salt(0x9E37_79B9_7F4A_7C15);
    let lo = hash_with_salt(0xC2B2_AE35_79B9_7F4A);
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    // Mark as RFC4122 variant with version nibble set for readability.
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).hyphenated().to_string()
}

fn make_block(
    role: Role,
    content: String,
    provider: &str,
    turn_index: u32,
    content_fp: &str,
    block_key: &str,
) -> Block {
    let tokens = estimate_tokens(&content);
    Block {
        id: stable_block_id(role, provider, content_fp, block_key),
        role,
        block_type: None,
        content: content.clone(),
        tokens,
        timestamp: now_iso8601(),
        zone: default_zone_for_role(role),
        pinned: None,
        compression_level: CompressionLevel::Original,
        compressed_versions: CompressionVersions {
            original: CompressionVersion { content, tokens },
            trimmed: None,
            summarized: None,
            minimal: None,
        },
        usage_heat: 0.0,
        position_relevance: 0.0,
        last_referenced_turn: turn_index,
        reference_count: 0,
        topic_cluster: None,
        topic_keywords: Vec::new(),
        metadata: BlockMetadata {
            provider: provider.to_string(),
            turn_index,
            tool_name: None,
            file_paths: Vec::new(),
        },
    }
}

fn make_tool_block(
    role: Role,
    content: String,
    provider: &str,
    turn_index: u32,
    tool_name: Option<String>,
    content_fp: &str,
    block_key: &str,
) -> Block {
    let mut block = make_block(role, content, provider, turn_index, content_fp, block_key);
    block.metadata.tool_name = tool_name;
    block
}

fn estimate_request_overhead(raw: &serde_json::Value) -> u32 {
    overhead::estimate_request_overhead(raw)
}

fn derive_thread_identity(raw: &serde_json::Value, blocks: &[Block]) -> Option<String> {
    identity::derive_thread_identity(raw, blocks)
}

fn short_hash(input: &str) -> String {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Compute a content fingerprint from the first 200 characters.
///
/// Used in block ID generation so that IDs are content-stable rather than
/// position-dependent. Two blocks with the same content produce the same
/// fingerprint regardless of their position in the message array.
fn content_fingerprint(content: &str) -> String {
    short_hash(&content.chars().take(200).collect::<String>())
}

/// Tracks how many times each (role, fingerprint) pair has been seen.
///
/// When multiple blocks share the same role and content fingerprint (e.g.
/// identical user messages), the occurrence index disambiguates their IDs
/// without relying on array position.
struct OccurrenceTracker {
    counts: std::collections::HashMap<String, u32>,
}

impl OccurrenceTracker {
    fn new() -> Self {
        Self {
            counts: std::collections::HashMap::new(),
        }
    }

    /// Return the next occurrence index for this (role, fingerprint) pair.
    fn next(&mut self, role: Role, fingerprint: &str) -> u32 {
        let key = format!("{role:?}|{fingerprint}");
        let count = self.counts.entry(key).or_insert(0);
        let current = *count;
        *count += 1;
        current
    }
}
/// Default zone assignment based on role.
fn default_zone_for_role(role: Role) -> Zone {
    use crate::engine::types::BuiltInZone;
    match role {
        Role::System => Zone::BuiltIn(BuiltInZone::Primacy),
        _ => Zone::BuiltIn(BuiltInZone::Recency),
    }
}
// ============================================================================
// Unified parser dispatch
// ============================================================================

/// Parse a request body, auto-detecting format from the path.
pub fn parse_request(path: &str, body: &[u8]) -> Result<ParsedRequest, String> {
    if body.is_empty() {
        return Err("Empty request body".to_string());
    }

    if is_messages_path(path) {
        parse_anthropic_request(body)
    } else if is_chat_completions_path(path) {
        parse_openai_chat_request(body)
    } else if is_responses_path(path) {
        parse_openai_responses_request(body)
    } else {
        // Try Anthropic first (primary use case), fall back to OpenAI
        parse_anthropic_request(body).or_else(|_| parse_openai_chat_request(body))
    }
}

/// Parse a request body into blocks only (pre-rewrite extraction).
///
/// Thin wrapper around `parse_request` that returns just the block list.
/// Used to capture block IDs from the ORIGINAL request body before any
/// rewriting, so the planner can detect re-sent archived content.
pub fn parse_request_blocks(
    path: &str,
    body: &[u8],
) -> Result<Vec<crate::engine::block::Block>, String> {
    parse_request(path, body).map(|parsed| parsed.blocks)
}

/// Parse a response body, using the known provider.
pub fn parse_response(
    provider: Provider,
    path: &str,
    body: &[u8],
) -> Result<ParsedResponse, String> {
    if body.is_empty() {
        return Ok(ParsedResponse {
            provider,
            blocks: Vec::new(),
            usage: None,
            model: None,
        });
    }

    match provider {
        Provider::Anthropic => parse_anthropic_response(body),
        Provider::OpenAI => {
            if is_responses_path(path) {
                parse_openai_responses_response(body)
            } else {
                parse_openai_chat_response(body)
            }
        }
    }
}

#[cfg(test)]
mod tests;
