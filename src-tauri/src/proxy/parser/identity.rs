use crate::engine::block::Block;
use crate::engine::types::Role;

use super::short_hash;

fn normalize_identity_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let compact = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        None
    } else {
        Some(compact.chars().take(256).collect())
    }
}

fn normalize_anchor_content(value: &str, max_chars: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let compact = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        None
    } else {
        Some(compact.chars().take(max_chars).collect())
    }
}

fn is_transient_user_anchor(value: &str) -> bool {
    let trimmed = value.trim_start();
    trimmed.starts_with("<system-reminder>")
        || trimmed.starts_with("<local-command-caveat>")
        || trimmed.starts_with("<local-command-stdout>")
        || trimmed.starts_with("<command-name>")
        || trimmed.starts_with("<command-message>")
        || trimmed.starts_with("<command-args>")
}

fn sanitize_system_anchor(value: &str) -> Option<String> {
    let filtered = value
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start().to_ascii_lowercase();
            !trimmed.starts_with("x-anthropic-billing-header:")
        })
        .collect::<Vec<_>>()
        .join(" ");
    normalize_anchor_content(&filtered, 160)
}

fn extract_nested_identity(raw: &serde_json::Value, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(|v| v.as_str())
        .and_then(normalize_identity_value)
        .or_else(|| {
            raw.get("metadata")
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .and_then(normalize_identity_value)
        })
        .or_else(|| {
            raw.get("context")
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .and_then(normalize_identity_value)
        })
}

fn explicit_thread_identity(raw: &serde_json::Value) -> Option<String> {
    for key in [
        "thread_id",
        "threadId",
        "session_id",
        "sessionId",
        "conversation_id",
        "conversationId",
        "previous_response_id",
        "previousResponseId",
        "response_id",
        "responseId",
    ] {
        if let Some(value) = extract_nested_identity(raw, key) {
            return Some(value);
        }
    }

    if let Some(conversation) = raw.get("conversation") {
        if let Some(value) = conversation.as_str().and_then(normalize_identity_value) {
            return Some(value);
        }
        if let Some(value) = conversation
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(normalize_identity_value)
        {
            return Some(value);
        }
    }

    None
}

fn fallback_thread_identity(blocks: &[Block]) -> Option<String> {
    let mut anchors = Vec::new();

    // Prefer stable conversational anchors. Claude often injects transient wrappers
    // (`<system-reminder>`, local command caveats), and system headers can include
    // per-turn billing metadata.
    if let Some(first_user) = blocks
        .iter()
        .find(|b| b.role == Role::User && !is_transient_user_anchor(&b.content))
        .and_then(|b| normalize_anchor_content(&b.content, 160))
    {
        anchors.push(format!("user:{first_user}"));
    }
    if let Some(first_assistant) = blocks
        .iter()
        .find(|b| b.role == Role::Assistant)
        .and_then(|b| normalize_anchor_content(&b.content, 120))
    {
        anchors.push(format!("assistant:{first_assistant}"));
    }

    // Fallback for sparse payloads.
    if anchors.is_empty() {
        if let Some(system) = blocks
            .iter()
            .find(|b| b.role == Role::System)
            .and_then(|b| sanitize_system_anchor(&b.content))
        {
            anchors.push(format!("system:{system}"));
        }
    }
    if anchors.is_empty() {
        if let Some(first_user_any) = blocks
            .iter()
            .find(|b| b.role == Role::User)
            .and_then(|b| normalize_anchor_content(&b.content, 160))
        {
            anchors.push(format!("user:{first_user_any}"));
        }
    }
    if anchors.is_empty() {
        if let Some(first_assistant_any) = blocks
            .iter()
            .find(|b| b.role == Role::Assistant)
            .and_then(|b| normalize_anchor_content(&b.content, 120))
        {
            anchors.push(format!("assistant:{first_assistant_any}"));
        }
    }
    if anchors.is_empty() {
        if let Some(system_any) = blocks
            .iter()
            .find(|b| b.role == Role::System)
            .and_then(|b| normalize_anchor_content(&b.content, 160))
        {
            anchors.push(format!("system:{system_any}"));
        }
    }
    if anchors.is_empty() {
        if let Some(first_any) = blocks
            .iter()
            .find(|b| !b.content.trim().is_empty())
            .and_then(|b| normalize_anchor_content(&b.content, 160))
        {
            anchors.push(format!("any:{first_any}"));
        }
    }

    (!anchors.is_empty()).then(|| format!("fallback:{}", short_hash(&anchors.join("|"))))
}

pub(super) fn derive_thread_identity(raw: &serde_json::Value, blocks: &[Block]) -> Option<String> {
    explicit_thread_identity(raw).or_else(|| fallback_thread_identity(blocks))
}
