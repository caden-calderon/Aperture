//! Aperture - Universal LLM context visualization, management, and control proxy.
//!
//! Aperture is a transparent proxy that sits between AI coding tools (Claude Code,
//! Codex, OpenCode, etc.) and their upstream APIs. It captures and visualizes
//! context without requiring any API keys of its own — the tools' existing
//! credentials pass through transparently.

pub mod engine;
pub mod events;
pub mod proxy;
pub mod terminal;
pub mod util;

use std::env;
use std::sync::Arc;

use tauri::Manager;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use engine::ContextEngine;
use events::dispatcher::{DynDispatcher, EventDispatcher};
use proxy::hot_patch::{HotPatch, HotPatchQueue, PatchSource};

/// Load environment from .env file (if present).
fn load_env() {
    // Try to load from project root first, then current dir
    let paths = [".env", "../.env", "../../.env"];
    for path in paths {
        if dotenvy::from_filename(path).is_ok() {
            return;
        }
    }
    // No .env found is fine — not required
}

/// Initialize logging with tracing.
fn init_logging() {
    let default_filter = if cfg!(debug_assertions) {
        // Development default: keep crate-level debug visibility for proxy diagnostics.
        "info,aperture_lib=debug"
    } else {
        // Release default: info level without verbose payload previews.
        "info"
    };

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| default_filter.into()))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Get configured proxy port from environment or default.
fn get_proxy_port() -> u16 {
    env::var("APERTURE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(proxy::DEFAULT_PORT)
}

// ── Proxy Commands ───────────────────────────────────────────

/// Tauri command: Get the proxy server address.
#[tauri::command]
fn get_proxy_address() -> String {
    format!("http://127.0.0.1:{}", get_proxy_port())
}

/// Tauri command: Check if proxy is running via TCP health check.
#[tauri::command]
fn is_proxy_running() -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let port = get_proxy_port();
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

// ── Hot Patch Commands ───────────────────────────────────────

/// Tauri command: Queue a hot patch to apply on the next outbound request.
#[tauri::command]
fn queue_hot_patch(
    state: tauri::State<'_, Arc<HotPatchQueue>>,
    role: String,
    original_content: String,
    new_content: String,
) {
    state.enqueue(HotPatch {
        role,
        original_content,
        new_content,
        source: PatchSource::Manual,
    });
}

/// Tauri command: Clear all pending hot patches.
#[tauri::command]
fn clear_hot_patches(state: tauri::State<'_, Arc<HotPatchQueue>>) {
    state.clear();
}

/// Tauri command: Get the number of pending hot patches.
#[tauri::command]
fn pending_hot_patch_count(state: tauri::State<'_, Arc<HotPatchQueue>>) -> usize {
    state.len()
}

// ── Engine Commands ──────────────────────────────────────────

/// Tauri command: Get all blocks from the active session.
#[tauri::command]
fn engine_get_blocks(state: tauri::State<'_, Arc<ContextEngine>>) -> Vec<engine::block::Block> {
    state.active_session_blocks()
}

/// Tauri command: Get a single block by ID.
#[tauri::command]
fn engine_get_block(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
) -> Option<engine::block::Block> {
    state.block(&block_id)
}

/// Tauri command: List all sessions.
#[tauri::command]
fn engine_get_sessions(
    state: tauri::State<'_, Arc<ContextEngine>>,
) -> Vec<engine::session::SessionInfo> {
    state.list_sessions()
}

/// Tauri command: Get the active session info.
#[tauri::command]
fn engine_get_active_session(
    state: tauri::State<'_, Arc<ContextEngine>>,
) -> Option<engine::session::SessionInfo> {
    state.active_session()
}

/// Tauri command: Get budget status for the active session.
#[tauri::command]
fn engine_get_budget_status(
    state: tauri::State<'_, Arc<ContextEngine>>,
) -> engine::budget::BudgetStatus {
    state.budget_status()
}

/// Tauri command: Get version history for a block.
#[tauri::command]
fn engine_get_block_versions(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
) -> Vec<engine::versioning::BlockVersion> {
    state.block_versions(&block_id)
}

/// Tauri command: Get recent action log entries.
#[tauri::command]
fn engine_get_recent_actions(
    state: tauri::State<'_, Arc<ContextEngine>>,
    count: usize,
) -> Vec<engine::action_log::ActionRecord> {
    state.recent_actions(count)
}

/// Tauri command: Update block content (goes through policy + versioning).
#[tauri::command]
fn engine_update_content(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
    content: String,
    model: String,
    confirmed: Option<bool>,
) -> Result<String, String> {
    let decision = state.update_content(&block_id, &content, &model, confirmed.unwrap_or(false))?;
    Ok(serde_json::to_string(&decision).unwrap_or_default())
}

/// Tauri command: Move a block to a different zone.
#[tauri::command]
fn engine_move_block(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
    zone: String,
    confirmed: Option<bool>,
) -> Result<String, String> {
    let target_zone = parse_zone(&zone);
    let decision = state.move_block(&block_id, target_zone, confirmed.unwrap_or(false))?;
    Ok(serde_json::to_string(&decision).unwrap_or_default())
}

/// Tauri command: Move multiple blocks to a different zone atomically.
#[tauri::command]
fn engine_bulk_move(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_ids: Vec<String>,
    zone: String,
    confirmed: Option<bool>,
) -> Result<String, String> {
    let target_zone = parse_zone(&zone);
    let decision = state.bulk_move(&block_ids, target_zone, confirmed.unwrap_or(false))?;
    Ok(serde_json::to_string(&decision).unwrap_or_default())
}

/// Tauri command: Remove a block.
#[tauri::command]
fn engine_remove_block(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
    confirmed: Option<bool>,
) -> Result<String, String> {
    let decision = state.remove_block(&block_id, confirmed.unwrap_or(false))?;
    Ok(serde_json::to_string(&decision).unwrap_or_default())
}

/// Tauri command: Remove multiple blocks atomically.
#[tauri::command]
fn engine_bulk_remove(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_ids: Vec<String>,
    confirmed: Option<bool>,
) -> Result<String, String> {
    let decision = state.bulk_remove(&block_ids, confirmed.unwrap_or(false))?;
    Ok(serde_json::to_string(&decision).unwrap_or_default())
}

/// Tauri command: Switch to a different session.
#[tauri::command]
fn engine_switch_session(state: tauri::State<'_, Arc<ContextEngine>>, session_id: String) -> bool {
    state.switch_session(&session_id)
}

/// Tauri command: Undo the last edit on a block.
#[tauri::command]
fn engine_undo_block(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
) -> Result<(), String> {
    state.undo_block(&block_id)
}

/// Tauri command: Pin or unpin a block.
#[tauri::command]
fn engine_pin_block(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
    position: Option<String>,
) -> Result<String, String> {
    let pin_pos = match position.as_deref() {
        Some("top") => Some(engine::types::PinPosition::Top),
        Some("bottom") => Some(engine::types::PinPosition::Bottom),
        None | Some("null") => None,
        Some(other) => return Err(format!("Invalid pin position: {other}")),
    };
    let decision = state.pin_block(&block_id, pin_pos)?;
    Ok(serde_json::to_string(&decision).unwrap_or_default())
}

/// Tauri command: Compress a block to a specified level.
#[tauri::command]
fn engine_compress_block(
    state: tauri::State<'_, Arc<ContextEngine>>,
    block_id: String,
    level: String,
    confirmed: Option<bool>,
) -> Result<String, String> {
    let compression_level = match level.as_str() {
        "original" => engine::types::CompressionLevel::Original,
        "trimmed" => engine::types::CompressionLevel::Trimmed,
        "summarized" => engine::types::CompressionLevel::Summarized,
        "minimal" => engine::types::CompressionLevel::Minimal,
        other => return Err(format!("Invalid compression level: {other}")),
    };
    let decision =
        state.compress_block(&block_id, compression_level, confirmed.unwrap_or(false))?;
    Ok(serde_json::to_string(&decision).unwrap_or_default())
}

/// Parse a zone string into a Zone enum.
fn parse_zone(zone: &str) -> engine::types::Zone {
    use engine::types::{BuiltInZone, Zone};
    match zone {
        "primacy" => Zone::BuiltIn(BuiltInZone::Primacy),
        "middle" => Zone::BuiltIn(BuiltInZone::Middle),
        "recency" => Zone::BuiltIn(BuiltInZone::Recency),
        custom => Zone::Custom(custom.to_string()),
    }
}

// ── App Entry ────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load .env before anything else
    load_env();

    init_logging();

    let port = get_proxy_port();

    info!("Starting Aperture");
    info!("Transparent proxy mode — tools' API keys pass through, no Aperture key needed");

    // Shared hot patch queue — accessible from both Tauri commands and proxy handler
    let hot_patch_queue = Arc::new(HotPatchQueue::new());
    let hot_patch_for_proxy = hot_patch_queue.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(terminal::TerminalState::new())
        .manage(hot_patch_queue)
        .setup(move |app| {
            // Create event dispatcher from the app handle
            let dispatcher = EventDispatcher::new(app.handle().clone());
            let dyn_dispatcher = DynDispatcher::from_typed(dispatcher);

            // Create context engine with a clone of the dispatcher
            let engine_dispatcher = dyn_dispatcher.clone();
            let engine = Arc::new(ContextEngine::new(Some(engine_dispatcher)));
            let engine_for_proxy = engine.clone();

            // Register engine as Tauri managed state
            app.manage(engine);

            // Start proxy server in background with event dispatch + hot patch queue + engine
            std::thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        error!("Failed to create tokio runtime: {}", e);
                        return;
                    }
                };
                rt.block_on(async move {
                    if let Err(e) = proxy::start_proxy(
                        port,
                        Some(dyn_dispatcher),
                        Some(hot_patch_for_proxy),
                        Some(engine_for_proxy),
                    )
                    .await
                    {
                        error!("Proxy server error: {}", e);
                    }
                });
            });

            info!("Proxy listening on http://127.0.0.1:{}", port);
            info!("Usage: ANTHROPIC_BASE_URL=http://localhost:{} claude", port);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Proxy
            get_proxy_address,
            is_proxy_running,
            // Hot patches
            queue_hot_patch,
            clear_hot_patches,
            pending_hot_patch_count,
            // Terminal
            terminal::spawn_shell,
            terminal::spawn_provider_shell,
            terminal::start_codex_subscription_bridge,
            terminal::stop_codex_subscription_bridge,
            terminal::is_codex_subscription_bridge_running,
            terminal::send_input,
            terminal::resize_terminal,
            terminal::kill_session,
            // Engine queries
            engine_get_blocks,
            engine_get_block,
            engine_get_sessions,
            engine_get_active_session,
            engine_get_budget_status,
            engine_get_block_versions,
            engine_get_recent_actions,
            // Engine mutations
            engine_update_content,
            engine_move_block,
            engine_bulk_move,
            engine_remove_block,
            engine_bulk_remove,
            engine_pin_block,
            engine_compress_block,
            engine_switch_session,
            engine_undo_block,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
