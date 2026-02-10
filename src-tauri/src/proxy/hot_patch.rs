//! Hot patch mode — queue edits to apply on the next outbound request.
//!
//! When a user edits a block in the UI, the edit is queued as a `HotPatch`.
//! On the next outbound API request, all pending patches are applied to the
//! request body before forwarding. Patches match messages by role and content.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tracing::{debug, info, warn};

/// A pending edit to apply to the next outbound request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotPatch {
    /// Target message role (e.g. "system", "user", "assistant").
    pub role: String,
    /// Original content to match (substring match — must be unique enough).
    pub original_content: String,
    /// Replacement content.
    pub new_content: String,
    /// Where this edit came from.
    pub source: PatchSource,
}

/// Origin of a hot patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchSource {
    /// User manually edited a block in the UI.
    Manual,
    /// Applied by an automated rule.
    Rule,
}

/// Thread-safe queue of pending hot patches.
#[derive(Debug, Default)]
pub struct HotPatchQueue {
    patches: Mutex<Vec<HotPatch>>,
}

impl HotPatchQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a patch to the queue.
    pub fn enqueue(&self, patch: HotPatch) {
        if let Ok(mut q) = self.patches.lock() {
            info!(
                "Hot patch queued: role={} content_len={} -> {}",
                patch.role,
                patch.original_content.len(),
                patch.new_content.len()
            );
            q.push(patch);
        }
    }

    /// Drain all pending patches (returns them and clears the queue).
    pub fn drain(&self) -> Vec<HotPatch> {
        self.patches
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    /// Clone all pending patches without removing them.
    ///
    /// Patches persist and re-apply on every outbound request until
    /// explicitly cleared. This is critical because LLM tools (Claude Code,
    /// Codex) re-send their own conversation history on each request —
    /// a one-shot drain would let the edit revert on the next exchange.
    pub fn peek_all(&self) -> Vec<HotPatch> {
        self.patches.lock().map(|q| q.clone()).unwrap_or_default()
    }

    /// Number of pending patches.
    pub fn len(&self) -> usize {
        self.patches.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all pending patches without applying them.
    pub fn clear(&self) {
        if let Ok(mut q) = self.patches.lock() {
            q.clear();
        }
    }
}

/// Apply pending patches to a JSON request body.
///
/// Modifies the `messages` array (Anthropic/OpenAI Chat) or `input` array
/// (OpenAI Responses) in-place. Returns the modified body bytes if any
/// patches were applied, or `None` if no changes were made.
pub fn apply_patches(body: &[u8], patches: &[HotPatch]) -> Option<Vec<u8>> {
    if patches.is_empty() {
        return None;
    }

    let mut json: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                "Hot patch: failed to parse request body as JSON ({e}), \
                 body_len={} — is the body compressed?",
                body.len()
            );
            return None;
        }
    };
    let mut applied = 0;

    // Try Anthropic system prompt (top-level "system" field)
    if let Some(system_val) = json.get_mut("system") {
        for patch in patches.iter().filter(|p| p.role == "system") {
            if let Some(s) = system_val.as_str() {
                if s.contains(&patch.original_content) {
                    let replaced = s.replace(&patch.original_content, &patch.new_content);
                    *system_val = serde_json::Value::String(replaced);
                    applied += 1;
                    debug!("Hot patch applied to system prompt");
                }
            }
        }
    }

    // Try messages array (Anthropic + OpenAI Chat)
    if let Some(messages) = json.get_mut("messages").and_then(|v| v.as_array_mut()) {
        applied += apply_to_messages(messages, patches);
    }

    // Try input array (OpenAI Responses)
    if let Some(input) = json.get_mut("input").and_then(|v| v.as_array_mut()) {
        applied += apply_to_messages(input, patches);
    }

    if applied > 0 {
        info!("{applied} hot patch(es) applied to outbound request");
        serde_json::to_vec(&json).ok()
    } else {
        debug!("No hot patches matched the outbound request");
        None
    }
}

/// Apply patches to a messages array in-place. Returns count of applied patches.
fn apply_to_messages(messages: &mut [serde_json::Value], patches: &[HotPatch]) -> usize {
    let mut applied = 0;

    for msg in messages.iter_mut() {
        // Clone role string to release immutable borrow before mutating msg
        let msg_role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();

        for patch in patches {
            if patch.role != msg_role {
                continue;
            }

            // Try string content
            if let Some(content) = msg
                .get("content")
                .and_then(|c| c.as_str().map(String::from))
            {
                if content.contains(&patch.original_content) {
                    let replaced = content.replace(&patch.original_content, &patch.new_content);
                    msg["content"] = serde_json::Value::String(replaced);
                    applied += 1;
                    debug!("Hot patch applied to message role={msg_role}");
                    continue;
                }
            }

            // Try array content (Anthropic content blocks)
            if let Some(content_arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                for block in content_arr.iter_mut() {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str().map(String::from))
                    {
                        if text.contains(&patch.original_content) {
                            let replaced =
                                text.replace(&patch.original_content, &patch.new_content);
                            block["text"] = serde_json::Value::String(replaced);
                            applied += 1;
                            debug!("Hot patch applied to content block role={msg_role}");
                        }
                    }
                }
            }
        }
    }

    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_patch(role: &str, original: &str, new: &str) -> HotPatch {
        HotPatch {
            role: role.to_string(),
            original_content: original.to_string(),
            new_content: new.to_string(),
            source: PatchSource::Manual,
        }
    }

    #[test]
    fn test_queue_enqueue_and_drain() {
        let queue = HotPatchQueue::new();
        assert!(queue.is_empty());

        queue.enqueue(make_patch("user", "hello", "world"));
        queue.enqueue(make_patch("system", "old", "new"));
        assert_eq!(queue.len(), 2);

        let drained = queue.drain();
        assert_eq!(drained.len(), 2);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_queue_clear() {
        let queue = HotPatchQueue::new();
        queue.enqueue(make_patch("user", "a", "b"));
        queue.clear();
        assert!(queue.is_empty());
    }

    #[test]
    fn test_apply_patches_openai_chat() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "Hello world" }
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let patches = vec![make_patch("user", "Hello world", "Goodbye world")];
        let result = apply_patches(&body_bytes, &patches).unwrap();
        let modified: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(
            modified["messages"][1]["content"].as_str().unwrap(),
            "Goodbye world"
        );
    }

    #[test]
    fn test_apply_patches_anthropic_system() {
        let body = serde_json::json!({
            "model": "claude-3-opus",
            "system": "You are a coding assistant.",
            "messages": [
                { "role": "user", "content": "Help me" }
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let patches = vec![make_patch(
            "system",
            "coding assistant",
            "writing assistant",
        )];
        let result = apply_patches(&body_bytes, &patches).unwrap();
        let modified: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(
            modified["system"].as_str().unwrap(),
            "You are a writing assistant."
        );
    }

    #[test]
    fn test_apply_patches_anthropic_content_blocks() {
        let body = serde_json::json!({
            "model": "claude-3-opus",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Original text here" }
                    ]
                }
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let patches = vec![make_patch("user", "Original text", "Modified text")];
        let result = apply_patches(&body_bytes, &patches).unwrap();
        let modified: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(
            modified["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap(),
            "Modified text here"
        );
    }

    #[test]
    fn test_apply_patches_no_match_returns_none() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                { "role": "user", "content": "Hello" }
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let patches = vec![make_patch("assistant", "nonexistent", "replaced")];
        assert!(apply_patches(&body_bytes, &patches).is_none());
    }

    #[test]
    fn test_apply_patches_empty_patches_returns_none() {
        let body = b"{}";
        assert!(apply_patches(body, &[]).is_none());
    }

    #[test]
    fn test_apply_patches_openai_responses_input() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "input": [
                { "role": "user", "content": "Explain quantum computing" }
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let patches = vec![make_patch("user", "quantum computing", "machine learning")];
        let result = apply_patches(&body_bytes, &patches).unwrap();
        let modified: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(
            modified["input"][0]["content"].as_str().unwrap(),
            "Explain machine learning"
        );
    }
}
