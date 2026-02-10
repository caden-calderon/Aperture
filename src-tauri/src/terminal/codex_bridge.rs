use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::engine::block::{Block, BlockMetadata, CompressionVersion, CompressionVersions};
use crate::engine::types::{BuiltInZone, CompressionLevel, Role, Zone};
use crate::engine::ContextEngine;
use crate::events::types::{channels, ApertureEvent};

const CODEX_BASE_POLL_INTERVAL: Duration = Duration::from_millis(1500);
const CODEX_MAX_POLL_INTERVAL: Duration = Duration::from_millis(12000);
const LOOP_SLEEP_INTERVAL: Duration = Duration::from_millis(250);

pub struct CodexBridgeHandle {
    stop_tx: Sender<()>,
    join_handle: JoinHandle<()>,
}

impl CodexBridgeHandle {
    pub fn stop(self) {
        let _ = self.stop_tx.send(());
        let _ = self.join_handle.join();
    }
}

pub fn spawn(app: AppHandle, engine: Arc<ContextEngine>) -> Result<CodexBridgeHandle, String> {
    let (stop_tx, stop_rx) = mpsc::channel();
    let join_handle = thread::Builder::new()
        .name("codex-subscription-bridge".to_string())
        .spawn(move || run_loop(app, stop_rx, engine))
        .map_err(|e| format!("failed to spawn codex bridge thread: {e}"))?;

    Ok(CodexBridgeHandle {
        stop_tx,
        join_handle,
    })
}

fn run_loop(app: AppHandle, stop_rx: Receiver<()>, engine: Arc<ContextEngine>) {
    let history_path = match codex_history_path() {
        Some(path) => path,
        None => {
            warn!("Codex bridge disabled: HOME not set");
            return;
        }
    };

    let mut cursor = file_len_or_zero(&history_path);
    let mut active_session_id: Option<String> = None;
    let mut last_emitted_digest: Option<u64> = None;
    let mut poll_interval = CODEX_BASE_POLL_INTERVAL;
    let mut last_poll = Instant::now()
        .checked_sub(CODEX_BASE_POLL_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_error: Option<String> = None;

    info!("Codex subscription bridge started");

    loop {
        if stop_rx.try_recv().is_ok() {
            info!("Codex subscription bridge stopping");
            return;
        }

        match read_new_history_entries(&history_path, cursor) {
            Ok((entries, new_cursor)) => {
                cursor = new_cursor;
                for entry in entries {
                    let session_changed =
                        active_session_id.as_deref() != Some(entry.session_id.as_str());
                    active_session_id = Some(entry.session_id);
                    if session_changed {
                        // New active session should refresh quickly and emit fresh context once.
                        last_emitted_digest = None;
                        poll_interval = CODEX_BASE_POLL_INTERVAL;
                    }
                    let _ = entry;
                }
            }
            Err(e) => {
                let msg = format!("failed reading codex history: {e}");
                if last_error.as_deref() != Some(msg.as_str()) {
                    warn!("{msg}");
                    last_error = Some(msg);
                }
            }
        }

        let should_poll = last_poll.elapsed() >= poll_interval;
        if should_poll {
            last_poll = Instant::now();

            if let Some(ref session_id) = active_session_id {
                let fetch_start = Instant::now();
                match fetch_thread_blocks(session_id) {
                    Ok(blocks) if !blocks.is_empty() => {
                        let digest = digest_blocks(&blocks);
                        if Some(digest) != last_emitted_digest {
                            emit_codex_blocks(&app, session_id, blocks, &engine);
                            last_emitted_digest = Some(digest);
                            poll_interval = CODEX_BASE_POLL_INTERVAL;
                        } else {
                            poll_interval = next_poll_interval(poll_interval);
                        }
                        debug!(
                            session_id,
                            fetch_ms = fetch_start.elapsed().as_millis() as u64,
                            poll_interval_ms = poll_interval.as_millis() as u64,
                            "Codex bridge poll complete"
                        );
                        last_error = None;
                    }
                    Ok(_) => {
                        poll_interval = next_poll_interval(poll_interval);
                    }
                    Err(e) => {
                        poll_interval = next_poll_interval(poll_interval);
                        if last_error.as_deref() != Some(e.as_str()) {
                            warn!("Codex bridge fetch failed: {e}");
                            app.emit(
                                channels::APERTURE_EVENTS,
                                ApertureEvent::ProxyError {
                                    request_id: None,
                                    message: format!("Codex bridge error: {e}"),
                                },
                            )
                            .ok();
                            last_error = Some(e);
                        }
                    }
                }
            } else {
                // No active Codex session observed yet — check less aggressively.
                poll_interval = next_poll_interval(poll_interval);
            }
        }

        thread::sleep(LOOP_SLEEP_INTERVAL);
    }
}

fn next_poll_interval(current: Duration) -> Duration {
    current.saturating_mul(2).min(CODEX_MAX_POLL_INTERVAL)
}

fn emit_codex_blocks(
    app: &AppHandle,
    session_id: &str,
    blocks: Vec<Block>,
    engine: &ContextEngine,
) {
    let request_id = format!("codex-session-{session_id}-{}", unix_timestamp_secs());

    // Emit request_captured + blocks_captured for connection metadata
    let request_event = ApertureEvent::RequestCaptured {
        request_id: request_id.clone(),
        method: "thread/read".to_string(),
        path: "codex://subscription".to_string(),
        provider: "openai".to_string(),
    };
    let blocks_event = ApertureEvent::BlocksCaptured {
        request_id,
        provider: "openai".to_string(),
        model: "codex-subscription".to_string(),
        request_blocks: blocks.clone(),
        response_blocks: Vec::new(),
        input_tokens: None,
        output_tokens: None,
    };

    if let Err(e) = app.emit(channels::APERTURE_EVENTS, request_event) {
        debug!("Failed to emit codex request_captured: {e}");
    }
    if let Err(e) = app.emit(channels::APERTURE_EVENTS, blocks_event) {
        debug!("Failed to emit codex blocks_captured: {e}");
    }

    // Feed blocks to engine — engine emits context_updated itself.
    // All codex blocks go as request_blocks (the bridge reads the full thread).
    engine.ingest(
        "openai",
        "codex-subscription",
        "direct_cli_bridge",
        Some(session_id),
        blocks,
        Vec::new(),
    );
}

fn codex_history_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex/history.jsonl"))
}

fn file_len_or_zero(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

#[derive(Debug, Deserialize)]
struct CodexHistoryEntry {
    session_id: String,
}

fn read_new_history_entries(
    history_path: &Path,
    mut cursor: u64,
) -> Result<(Vec<CodexHistoryEntry>, u64), String> {
    let file = match File::open(history_path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), cursor)),
        Err(e) => return Err(e.to_string()),
    };

    let current_len = file
        .metadata()
        .map_err(|e| format!("metadata failed: {e}"))?
        .len();
    if current_len < cursor {
        cursor = current_len;
    }
    if current_len == cursor {
        return Ok((Vec::new(), cursor));
    }

    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(cursor))
        .map_err(|e| format!("seek failed: {e}"))?;

    let mut line = String::new();
    let mut entries = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| format!("read_line failed: {e}"))?;
        if read == 0 {
            break;
        }
        cursor += read as u64;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<CodexHistoryEntry>(trimmed) {
            entries.push(entry);
        }
    }

    Ok((entries, cursor))
}

fn fetch_thread_blocks(session_id: &str) -> Result<Vec<Block>, String> {
    let init_request = json!({
        "id": "init-1",
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "aperture-codex-bridge",
                "title": "Aperture Codex Bridge",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "experimentalApi": true
            }
        }
    });
    let thread_read_request = json!({
        "id": "thread-read-1",
        "method": "thread/read",
        "params": {
            "threadId": session_id,
            "includeTurns": true
        }
    });

    let mut child = Command::new("codex")
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start codex app-server: {e}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        writeln!(stdin, "{init_request}")
            .map_err(|e| format!("failed writing initialize request: {e}"))?;
        writeln!(stdin, "{thread_read_request}")
            .map_err(|e| format!("failed writing thread/read request: {e}"))?;
    } else {
        return Err("failed to open codex app-server stdin".to_string());
    }
    // Close stdin so app-server can exit after handling requests.
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed waiting for codex app-server: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut thread_json: Option<Value> = None;
    let mut rpc_error: Option<String> = None;

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        if value.get("id").and_then(Value::as_str) == Some("thread-read-1") {
            if let Some(error) = value.get("error") {
                rpc_error = Some(error.to_string());
                break;
            }
            if let Some(thread) = value.get("result").and_then(|r| r.get("thread")).cloned() {
                thread_json = Some(thread);
                break;
            }
        }
    }

    if let Some(error) = rpc_error {
        return Err(format!("thread/read failed: {error}"));
    }
    if let Some(thread) = thread_json {
        return Ok(extract_blocks_from_thread_json(&thread));
    }

    if !stderr.trim().is_empty() {
        return Err(format!(
            "codex app-server produced no thread/read response: {stderr}"
        ));
    }
    Err("codex app-server produced no thread/read response".to_string())
}

fn extract_blocks_from_thread_json(thread: &Value) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut turn_index = 0u32;

    let Some(turns) = thread.get("turns").and_then(Value::as_array) else {
        return blocks;
    };

    for turn in turns {
        let Some(items) = turn.get("items").and_then(Value::as_array) else {
            turn_index = turn_index.saturating_add(1);
            continue;
        };

        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("userMessage") => {
                    let content = extract_user_message_text(item);
                    if !content.trim().is_empty() {
                        blocks.push(make_block(Role::User, content, turn_index));
                    }
                }
                Some("agentMessage") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            blocks.push(make_block(Role::Assistant, text.to_string(), turn_index));
                        }
                    }
                }
                Some("reasoning") => {
                    let mut text_parts: Vec<String> = Vec::new();
                    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                        for part in summary {
                            if let Some(s) = part.as_str() {
                                text_parts.push(s.to_string());
                            }
                        }
                    }
                    if let Some(content) = item.get("content").and_then(Value::as_array) {
                        for part in content {
                            if let Some(s) = part.as_str() {
                                text_parts.push(s.to_string());
                            }
                        }
                    }
                    let combined = text_parts.join("\n");
                    if !combined.trim().is_empty() {
                        blocks.push(make_block(Role::Thinking, combined, turn_index));
                    }
                }
                _ => {}
            }
        }

        turn_index = turn_index.saturating_add(1);
    }

    blocks
}

fn extract_user_message_text(item: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for entry in content {
            let entry_type = entry.get("type").and_then(Value::as_str);
            if entry_type == Some("text") {
                if let Some(text) = entry.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
        }
    }
    parts.join("\n")
}

fn digest_blocks(blocks: &[Block]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for block in blocks {
        block.role.hash(&mut hasher);
        block.content.hash(&mut hasher);
        block.tokens.hash(&mut hasher);
        block.metadata.turn_index.hash(&mut hasher);
    }
    hasher.finish()
}

fn estimate_tokens(content: &str) -> u32 {
    (content.len() as f64 / 4.0).ceil() as u32
}

fn unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_iso8601() -> String {
    crate::util::iso_now()
}

fn make_block(role: Role, content: String, turn_index: u32) -> Block {
    let tokens = estimate_tokens(&content);
    Block {
        id: Uuid::new_v4().to_string(),
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
            provider: "openai".to_string(),
            turn_index,
            tool_name: None,
            file_paths: Vec::new(),
        },
    }
}

fn default_zone_for_role(role: Role) -> Zone {
    match role {
        Role::System => Zone::BuiltIn(BuiltInZone::Primacy),
        _ => Zone::BuiltIn(BuiltInZone::Recency),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_blocks_from_thread_json_user_and_agent() {
        let thread = json!({
            "id": "thread-1",
            "turns": [
                {
                    "id": "turn-1",
                    "items": [
                        {
                            "type": "userMessage",
                            "id": "u1",
                            "content": [{ "type": "text", "text": "hello codex" }]
                        },
                        {
                            "type": "agentMessage",
                            "id": "a1",
                            "text": "hello human"
                        }
                    ]
                }
            ]
        });

        let blocks = extract_blocks_from_thread_json(&thread);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].role, Role::User);
        assert_eq!(blocks[0].content, "hello codex");
        assert_eq!(blocks[1].role, Role::Assistant);
        assert_eq!(blocks[1].content, "hello human");
    }

    #[test]
    fn test_extract_blocks_from_thread_json_reasoning_maps_to_thinking() {
        let thread = json!({
            "id": "thread-2",
            "turns": [
                {
                    "id": "turn-1",
                    "items": [
                        {
                            "type": "reasoning",
                            "id": "r1",
                            "summary": ["step 1"],
                            "content": ["detail 1"]
                        }
                    ]
                }
            ]
        });
        let blocks = extract_blocks_from_thread_json(&thread);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].role, Role::Thinking);
        assert!(blocks[0].content.contains("step 1"));
        assert!(blocks[0].content.contains("detail 1"));
    }

    #[test]
    fn test_read_new_history_entries_returns_entries_after_cursor() {
        let path =
            std::env::temp_dir().join(format!("aperture-codex-history-{}.jsonl", Uuid::new_v4()));
        std::fs::write(
            &path,
            r#"{"session_id":"s1","ts":1,"text":"a"}"#.to_string() + "\n",
        )
        .expect("write history");

        let (entries, cursor) = read_new_history_entries(&path, 0).expect("read history");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session_id, "s1");
        assert!(cursor > 0);

        let (entries2, cursor2) = read_new_history_entries(&path, cursor).expect("read history");
        assert!(entries2.is_empty());
        assert_eq!(cursor2, cursor);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_next_poll_interval_doubles_until_cap() {
        let first = next_poll_interval(CODEX_BASE_POLL_INTERVAL);
        assert_eq!(first, Duration::from_millis(3000));

        let second = next_poll_interval(first);
        assert_eq!(second, Duration::from_millis(6000));

        let capped = next_poll_interval(Duration::from_millis(9000));
        assert_eq!(capped, CODEX_MAX_POLL_INTERVAL);
    }
}
