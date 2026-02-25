//! HTTP IPC API for engine and hot-patch commands.
//!
//! Exposes the same operations that were previously Tauri IPC commands
//! as HTTP endpoints under `/_aperture/ipc/{command}`.
//!
//! ```text
//! POST /_aperture/ipc/{command}
//! Body: JSON arguments (same shape as Tauri invoke args)
//! Response: JSON result
//! ```

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::debug;

use super::ProxyState;
use crate::engine::types::{BuiltInZone, CompressionLevel, PinPosition, Zone};

const IPC_BODY_LIMIT: usize = 1024 * 64;

/// Dispatch an IPC command to the appropriate engine/hot-patch method.
pub async fn handle_ipc_command(
    state: &Arc<ProxyState>,
    command: &str,
    req: Request<Body>,
) -> Response {
    let args = match parse_body(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    debug!(command, "IPC command received");

    match command {
        // ── Engine Queries ────────────────────────────────────
        "engine_get_blocks" => with_engine(state, |e| json!(e.active_session_blocks())),
        "engine_get_block" => with_engine(state, |e| {
            let id = args["blockId"].as_str().unwrap_or("");
            json!(e.block(id))
        }),
        "engine_get_sessions" => with_engine(state, |e| json!(e.list_sessions())),
        "engine_get_active_session" => with_engine(state, |e| json!(e.active_session())),
        "engine_get_budget_status" => with_engine(state, |e| json!(e.budget_status())),
        "engine_get_block_versions" => with_engine(state, |e| {
            let id = args["blockId"].as_str().unwrap_or("");
            json!(e.block_versions(id))
        }),
        "engine_get_recent_actions" => with_engine(state, |e| {
            let count = args["count"].as_u64().unwrap_or(20) as usize;
            json!(e.recent_actions(count))
        }),
        "engine_get_budget_ceiling" => {
            with_engine(state, |e| json!(e.planner.effective_budget_ceiling()))
        }
        "engine_get_compression_settings" => {
            with_engine(state, |e| json!(e.compression_settings()))
        }

        // ── Engine Mutations ─────────────────────────────────
        "engine_update_content" => with_engine_result(state, |e| {
            let id = str_arg(&args, "blockId")?;
            let content = str_arg(&args, "content")?;
            let model = args["model"].as_str().unwrap_or("unknown");
            let confirmed = args["confirmed"].as_bool().unwrap_or(false);
            e.update_content(id, content, model, confirmed)
        }),
        "engine_move_block" => with_engine_result(state, |e| {
            let id = str_arg(&args, "blockId")?;
            let zone = zone_arg(&args, "zone")?;
            let confirmed = args["confirmed"].as_bool().unwrap_or(false);
            e.move_block(id, zone, confirmed)
        }),
        "engine_bulk_move" => with_engine_result(state, |e| {
            let ids = string_array_arg(&args, "blockIds")?;
            let zone = zone_arg(&args, "zone")?;
            let confirmed = args["confirmed"].as_bool().unwrap_or(false);
            e.bulk_move(&ids, zone, confirmed)
        }),
        "engine_remove_block" => with_engine_result(state, |e| {
            let id = str_arg(&args, "blockId")?;
            let confirmed = args["confirmed"].as_bool().unwrap_or(false);
            e.remove_block(id, confirmed)
        }),
        "engine_bulk_remove" => with_engine_result(state, |e| {
            let ids = string_array_arg(&args, "blockIds")?;
            let confirmed = args["confirmed"].as_bool().unwrap_or(false);
            e.bulk_remove(&ids, confirmed)
        }),
        "engine_pin_block" => with_engine_result(state, |e| {
            let id = str_arg(&args, "blockId")?;
            let pin_pos = match args["position"].as_str() {
                Some("top") => Some(PinPosition::Top),
                Some("bottom") => Some(PinPosition::Bottom),
                _ => None,
            };
            e.pin_block(id, pin_pos)
        }),
        "engine_compress_block" => with_engine_result(state, |e| {
            let id = str_arg(&args, "blockId")?;
            let level = compression_level_arg(&args, "level")?;
            let confirmed = args["confirmed"].as_bool().unwrap_or(false);
            e.compress_block(id, level, confirmed)
        }),
        "engine_switch_session" => with_engine(state, |e| {
            let id = args["sessionId"].as_str().unwrap_or("");
            json!(e.switch_session(id))
        }),
        "engine_undo_block" => {
            let engine = match state.engine.as_ref() {
                Some(e) => e,
                None => return engine_unavailable(),
            };
            let id = args["blockId"].as_str().unwrap_or("");
            match engine.undo_block(id) {
                Ok(()) => ok_json(json!(null)),
                Err(msg) => error_json(&msg),
            }
        }
        "engine_set_budget_ceiling" => with_engine(state, |e| {
            let ceiling = args["ceiling"].as_f64().unwrap_or(0.80);
            e.planner.set_budget_ceiling(ceiling);
            json!(null)
        }),
        "engine_update_compression_settings" => {
            let engine = match state.engine.as_ref() {
                Some(e) => e,
                None => return engine_unavailable(),
            };
            match serde_json::from_value(args["settings"].clone()) {
                Ok(settings) => {
                    engine.set_compression_settings(settings);
                    ok_json(json!(null))
                }
                Err(err) => error_json(&format!("invalid compression settings: {err}")),
            }
        }
        "engine_clear_context" => with_engine_result(state, |e| {
            let confirmed = args["confirmed"].as_bool().unwrap_or(false);
            e.clear_all_sessions(confirmed)
        }),

        // ── Hot Patches ──────────────────────────────────────
        "queue_hot_patch" => {
            let role = args["role"].as_str().unwrap_or("").to_string();
            let original = args["originalContent"].as_str().unwrap_or("").to_string();
            let new = args["newContent"].as_str().unwrap_or("").to_string();
            use super::hot_patch::{HotPatch, PatchSource};
            state.hot_patches.enqueue(HotPatch {
                role,
                original_content: original,
                new_content: new,
                source: PatchSource::Manual,
            });
            ok_json(json!(null))
        }
        "clear_hot_patches" => {
            state.hot_patches.clear();
            ok_json(json!(null))
        }
        "pending_hot_patch_count" => ok_json(json!(state.hot_patches.len())),

        // ── Proxy Info ───────────────────────────────────────
        "get_proxy_address" => {
            let port = std::env::var("APERTURE_PORT")
                .ok()
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(super::DEFAULT_PORT);
            ok_json(json!(format!("http://127.0.0.1:{port}")))
        }
        "is_proxy_running" => ok_json(json!(true)), // If we're handling this, we're running

        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("unknown IPC command: {command}")})),
        )
            .into_response(),
    }
}

// ── Helpers ──────────────────────────────────────────────────

async fn parse_body(req: Request<Body>) -> Result<Value, Response> {
    let bytes = axum::body::to_bytes(req.into_body(), IPC_BODY_LIMIT)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid body: {e}")})),
            )
                .into_response()
        })?;

    if bytes.is_empty() {
        return Ok(json!({}));
    }

    serde_json::from_slice(&bytes).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("invalid JSON: {e}")})),
        )
            .into_response()
    })
}

fn with_engine<F>(state: &Arc<ProxyState>, f: F) -> Response
where
    F: FnOnce(&crate::engine::ContextEngine) -> Value,
{
    match state.engine.as_ref() {
        Some(engine) => ok_json(f(engine)),
        None => engine_unavailable(),
    }
}

fn with_engine_result<F>(state: &Arc<ProxyState>, f: F) -> Response
where
    F: FnOnce(&crate::engine::ContextEngine) -> Result<crate::engine::policy::PolicyDecision, String>,
{
    let engine = match state.engine.as_ref() {
        Some(e) => e,
        None => return engine_unavailable(),
    };
    match f(engine) {
        Ok(decision) => ok_json(json!(decision)),
        Err(msg) => error_json(&msg),
    }
}

fn ok_json(value: Value) -> Response {
    (StatusCode::OK, Json(value)).into_response()
}

fn error_json(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": message})),
    )
        .into_response()
}

fn engine_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "context engine not available"})),
    )
        .into_response()
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

fn string_array_arg(args: &Value, key: &str) -> Result<Vec<String>, String> {
    args[key]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .ok_or_else(|| format!("missing required array argument: {key}"))
}

fn zone_arg(args: &Value, key: &str) -> Result<Zone, String> {
    let s = str_arg(args, key)?;
    Ok(parse_zone(s))
}

fn parse_zone(zone: &str) -> Zone {
    match zone {
        "primacy" => Zone::BuiltIn(BuiltInZone::Primacy),
        "middle" => Zone::BuiltIn(BuiltInZone::Middle),
        "recency" => Zone::BuiltIn(BuiltInZone::Recency),
        custom => Zone::Custom(custom.to_string()),
    }
}

fn compression_level_arg(args: &Value, key: &str) -> Result<CompressionLevel, String> {
    match args[key].as_str() {
        Some("original") => Ok(CompressionLevel::Original),
        Some("trimmed") => Ok(CompressionLevel::Trimmed),
        Some("summarized") => Ok(CompressionLevel::Summarized),
        Some("minimal") => Ok(CompressionLevel::Minimal),
        Some(other) => Err(format!("invalid compression level: {other}")),
        None => Err(format!("missing required argument: {key}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_zone_builtin() {
        assert!(matches!(parse_zone("primacy"), Zone::BuiltIn(BuiltInZone::Primacy)));
        assert!(matches!(parse_zone("middle"), Zone::BuiltIn(BuiltInZone::Middle)));
        assert!(matches!(parse_zone("recency"), Zone::BuiltIn(BuiltInZone::Recency)));
    }

    #[test]
    fn test_parse_zone_custom() {
        match parse_zone("my_zone") {
            Zone::Custom(name) => assert_eq!(name, "my_zone"),
            _ => panic!("expected custom zone"),
        }
    }

    #[test]
    fn test_compression_level_parsing() {
        let args = json!({"level": "summarized"});
        assert!(matches!(compression_level_arg(&args, "level"), Ok(CompressionLevel::Summarized)));

        let bad = json!({"level": "nonexistent"});
        assert!(compression_level_arg(&bad, "level").is_err());

        let missing = json!({});
        assert!(compression_level_arg(&missing, "level").is_err());
    }

    #[test]
    fn test_string_array_arg() {
        let args = json!({"ids": ["a", "b", "c"]});
        let result = string_array_arg(&args, "ids").unwrap();
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_string_array_arg_missing() {
        let args = json!({});
        assert!(string_array_arg(&args, "ids").is_err());
    }
}
