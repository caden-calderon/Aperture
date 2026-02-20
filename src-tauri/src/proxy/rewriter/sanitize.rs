use serde_json::Value;

/// Remove Anthropic `tool_result` blocks that don't match a `tool_use` in the
/// immediately preceding assistant message.
///
/// Anthropic requires each `tool_result.tool_use_id` to correspond to a
/// `tool_use.id` block in the previous message. If that invariant is broken,
/// the upstream API rejects the request.
pub(super) fn sanitize_anthropic_orphan_tool_results(request_json: &mut Value) -> usize {
    let Some(messages) = request_json
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
    else {
        return 0;
    };

    let mut removed = 0usize;

    for idx in 0..messages.len() {
        let Some(role) = messages[idx].get("role").and_then(|r| r.as_str()) else {
            continue;
        };
        if role != "user" {
            continue;
        }

        let valid_tool_use_ids: std::collections::HashSet<String> = if idx > 0 {
            let prev = &messages[idx - 1];
            if prev.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                prev.get("content")
                    .and_then(|c| c.as_array())
                    .map(|content| {
                        content
                            .iter()
                            .filter(|block| {
                                block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                            })
                            .filter_map(|block| {
                                block
                                    .get("id")
                                    .and_then(|id| id.as_str())
                                    .map(ToString::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                std::collections::HashSet::new()
            }
        } else {
            std::collections::HashSet::new()
        };

        let Some(content) = messages[idx]
            .get_mut("content")
            .and_then(|c| c.as_array_mut())
        else {
            continue;
        };

        let before = content.len();
        content.retain(|block| {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                return true;
            }
            let Some(tool_use_id) = block.get("tool_use_id").and_then(|id| id.as_str()) else {
                return false;
            };
            valid_tool_use_ids.contains(tool_use_id)
        });
        removed += before.saturating_sub(content.len());
    }

    if removed > 0 {
        // Cleanup any now-empty user messages.
        messages.retain(|message| {
            if let Some(content) = message.get("content").and_then(|c| c.as_array()) {
                !content.is_empty()
            } else {
                true
            }
        });
    }

    removed
}

/// Reverse-direction orphan sanitizer: strip `tool_use` blocks from assistant
/// messages when the immediately following user message lacks a matching
/// `tool_result` with the same `tool_use_id`.
///
/// Anthropic requires every assistant `tool_use` block to be paired with a
/// `tool_result` in the next user message. If context rewriting or archival
/// removes the user turn (or strips the `tool_result` from it), the orphaned
/// `tool_use` causes a 400 "tool use concurrency" error.
pub(super) fn sanitize_anthropic_orphan_tool_uses(request_json: &mut Value) -> usize {
    let Some(messages) = request_json
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
    else {
        return 0;
    };

    let mut removed = 0usize;

    for idx in 0..messages.len() {
        let Some(role) = messages[idx].get("role").and_then(|r| r.as_str()) else {
            continue;
        };
        if role != "assistant" {
            continue;
        }

        // Collect tool_result IDs from the next user message (if it exists).
        let valid_tool_result_ids: std::collections::HashSet<String> = if idx + 1 < messages.len() {
            let next = &messages[idx + 1];
            if next.get("role").and_then(|r| r.as_str()) == Some("user") {
                next.get("content")
                    .and_then(|c| c.as_array())
                    .map(|content| {
                        content
                            .iter()
                            .filter(|block| {
                                block.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                            })
                            .filter_map(|block| {
                                block
                                    .get("tool_use_id")
                                    .and_then(|id| id.as_str())
                                    .map(ToString::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                // Next message is not a user message - if this assistant has
                // tool_use blocks, they're all orphans (except for the last
                // message which may be awaiting a response).
                if idx + 1 == messages.len() {
                    // Last assistant message - don't strip, it's the pending turn.
                    continue;
                }
                std::collections::HashSet::new()
            }
        } else {
            // This assistant is the last message - pending turn, don't strip.
            continue;
        };

        let Some(content) = messages[idx]
            .get_mut("content")
            .and_then(|c| c.as_array_mut())
        else {
            continue;
        };

        let before = content.len();
        content.retain(|block| {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                return true;
            }
            let Some(id) = block.get("id").and_then(|id| id.as_str()) else {
                return false; // malformed tool_use - drop
            };
            valid_tool_result_ids.contains(id)
        });
        removed += before.saturating_sub(content.len());
    }

    if removed > 0 {
        // Cleanup any now-empty assistant messages.
        messages.retain(|message| {
            if let Some(content) = message.get("content").and_then(|c| c.as_array()) {
                !content.is_empty()
            } else {
                true
            }
        });
    }

    removed
}

/// Fix structural violations left by context tool cleanup.
///
/// After `strip_anthropic_context_tools()` removes tool_use/tool_result blocks,
/// user messages that contained ONLY tool_results become empty and are deleted.
/// This can leave:
/// 1. Consecutive assistant messages (Anthropic rejects these)
/// 2. The payload ending with an assistant message (Anthropic rejects as "prefill")
///
/// This sanitizer:
/// - Merges consecutive same-role messages by concatenating their content arrays
/// - Appends a synthetic user message if the last message is role=assistant
///
/// Returns the number of merges + synthetic messages added.
pub(super) fn sanitize_anthropic_message_structure(request_json: &mut Value) -> usize {
    let Some(messages) = request_json
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
    else {
        return 0;
    };

    if messages.is_empty() {
        return 0;
    }

    // Pass 1: Merge consecutive same-role messages.
    // Exception: never merge assistant messages containing thinking/redacted_thinking
    // blocks — Anthropic requires these to be byte-identical to the original response.
    let mut changes = 0usize;
    let mut write = 0usize;

    for read in 1..messages.len() {
        let prev_role = messages[write]
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        let cur_role = messages[read]
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();

        if prev_role == cur_role {
            // Skip merge if either assistant message contains thinking blocks
            if prev_role == "assistant"
                && (has_thinking_blocks(&messages[write])
                    || has_thinking_blocks(&messages[read]))
            {
                // Don't merge — advance write position and insert synthetic user between
                write += 1;
                if write != read {
                    messages.swap(write, read);
                }
            } else {
                // Merge current into write position.
                let cur_content = messages[read].get("content").cloned();
                merge_message_content(&mut messages[write], cur_content);
                changes += 1;
            }
        } else {
            write += 1;
            if write != read {
                messages.swap(write, read);
            }
        }
    }
    messages.truncate(write + 1);

    // Pass 1.5: Insert synthetic user messages between consecutive same-role messages
    // that were NOT merged (e.g. thinking-block assistant messages).
    let mut i = 1;
    while i < messages.len() {
        let prev_role = messages[i - 1]
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("");
        let cur_role = messages[i]
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("");

        if prev_role == cur_role {
            let synthetic = serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "Continue."}]
            });
            messages.insert(i, synthetic);
            changes += 1;
            i += 2; // skip past the inserted message
        } else {
            i += 1;
        }
    }

    // Pass 2: Ensure last message is role=user.
    if let Some(last) = messages.last() {
        if last.get("role").and_then(|r| r.as_str()) == Some("assistant") {
            let synthetic = serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "Continue."}]
            });
            messages.push(synthetic);
            changes += 1;
        }
    }

    changes
}

/// Merge source content into a target message's content array.
fn merge_message_content(target: &mut Value, source_content: Option<Value>) {
    let Some(source) = source_content else {
        return;
    };

    // Normalize both sides to arrays of content blocks.
    let source_blocks = match source {
        Value::Array(arr) => arr,
        Value::String(s) => {
            vec![serde_json::json!({"type": "text", "text": s})]
        }
        _ => return,
    };

    let target_content = target.get_mut("content");
    match target_content {
        Some(Value::Array(arr)) => {
            arr.extend(source_blocks);
        }
        Some(Value::String(existing)) => {
            // Convert string content to array, then append source blocks.
            let mut new_arr =
                vec![serde_json::json!({"type": "text", "text": existing.as_str()})];
            new_arr.extend(source_blocks);
            target["content"] = Value::Array(new_arr);
        }
        _ => {
            target["content"] = Value::Array(source_blocks);
        }
    }
}

/// Check if a message contains thinking or redacted_thinking content blocks.
fn has_thinking_blocks(message: &Value) -> bool {
    let Some(content) = message.get("content").and_then(|c| c.as_array()) else {
        return false;
    };
    content.iter().any(|block| {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        block_type == "thinking" || block_type == "redacted_thinking"
    })
}
