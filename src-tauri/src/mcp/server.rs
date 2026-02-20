use std::io::{self, BufRead, Write};

use crate::metacog::context_tool_definitions;
use serde_json::{json, Value};

/// Map tool names to their HTTP API path suffix.
pub(super) fn tool_api_path(name: &str) -> Option<&'static str> {
    match name {
        "aperture_context_preview" => Some("/context/preview"),
        "aperture_context_read" => Some("/context/read"),
        "aperture_context_search" => Some("/context/search"),
        "aperture_context_plan" => Some("/context/plan"),
        "aperture_context_status" => Some("/context/status"),
        _ => None,
    }
}

/// Build MCP tool definitions from shared runtime source-of-truth definitions.
pub(super) fn tool_definitions() -> Value {
    Value::Array(
        context_tool_definitions()
            .into_iter()
            .map(|def| {
                json!({
                    "name": def.name,
                    "description": def.description,
                    "inputSchema": def.parameters_schema
                })
            })
            .collect(),
    )
}

/// Write a JSON-RPC response to stdout.
fn send_response(id: &Value, result: Value) {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    });
    let out = serde_json::to_string(&msg).expect("serialize response");
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{out}").expect("write stdout");
    stdout.flush().expect("flush stdout");
}

/// Write a JSON-RPC error response to stdout.
fn send_error(id: &Value, code: i64, message: &str) {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    });
    let out = serde_json::to_string(&msg).expect("serialize error");
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{out}").expect("write stdout");
    stdout.flush().expect("flush stdout");
}

/// Call the Aperture proxy HTTP API for a tool invocation.
///
/// Retries once on transient connection failures (R9-3 fix) with a 500ms delay.
fn call_proxy(base_url: &str, api_path: &str, arguments: &Value) -> Result<Value, String> {
    let url = format!("{base_url}/_aperture{api_path}");
    let client = reqwest::blocking::Client::new();
    let max_attempts = 2;

    let mut last_err = String::new();
    for attempt in 1..=max_attempts {
        match client
            .post(&url)
            .json(arguments)
            .timeout(std::time::Duration::from_secs(30))
            .send()
        {
            Ok(resp) => {
                let status = resp.status();
                let body: Value = resp
                    .json()
                    .map_err(|e| format!("Failed to parse proxy response: {e}"))?;

                if !status.is_success() {
                    return Err(format!(
                        "Proxy returned {status}: {}",
                        body.get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("unknown error")
                    ));
                }

                return Ok(body);
            }
            Err(e) => {
                last_err = format!("HTTP request to proxy failed: {e}");
                if attempt < max_attempts {
                    eprintln!(
                        "aperture-mcp: call_proxy attempt {attempt} failed ({e}), retrying in 500ms"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
    }

    Err(last_err)
}

pub(super) fn infer_plan_control_op(arguments: &Value) -> &'static str {
    if let Some(op) = arguments
        .get("control")
        .and_then(|v| v.get("op"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return match op {
            "stage" => "stage",
            "append" => "append",
            "preview" => "preview",
            "commit" => "commit",
            "discard" => "discard",
            _ => "preview",
        };
    }

    // Mirror server-side default behavior: no explicit op + action payload => stage.
    let has_payload = arguments
        .as_object()
        .map(|obj| {
            obj.iter().any(|(k, v)| {
                if k == "control" || k == "_aperture_session_id" {
                    return false;
                }
                match v {
                    Value::Array(items) => !items.is_empty(),
                    Value::Object(map) => !map.is_empty(),
                    Value::Null => false,
                    _ => true,
                }
            })
        })
        .unwrap_or(false);

    if has_payload {
        "stage"
    } else {
        "preview"
    }
}

pub(super) fn with_plan_session_hint(
    arguments: &Value,
    op: &str,
    session_id: Option<&str>,
) -> Value {
    let mut payload = arguments.clone();
    let should_pin = matches!(op, "stage" | "append" | "preview" | "commit" | "discard");

    if should_pin {
        if let (Some(id), Some(obj)) = (session_id, payload.as_object_mut()) {
            obj.insert(
                "_aperture_session_id".to_string(),
                Value::String(id.to_string()),
            );
        }
    }

    payload
}

pub(super) fn update_plan_session_affinity(
    plan_session_affinity: &mut Option<String>,
    op: &str,
    is_error: bool,
    result: &Value,
) {
    if is_error {
        return;
    }

    match op {
        "commit" | "discard" => {
            *plan_session_affinity = None;
        }
        "stage" | "append" | "preview" => {
            if let Some(session_id) = result
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                *plan_session_affinity = Some(session_id.to_string());
            }
        }
        _ => {}
    }
}

/// Handle a tools/call request: forward to proxy, return MCP-format result.
fn handle_tools_call(
    id: &Value,
    params: &Value,
    base_url: &str,
    plan_session_affinity: &mut Option<String>,
) {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let api_path = match tool_api_path(name) {
        Some(p) => p,
        None => {
            send_error(id, -32602, &format!("Unknown tool: {name}"));
            return;
        }
    };

    let plan_op = if name == "aperture_context_plan" {
        infer_plan_control_op(&arguments)
    } else {
        "preview"
    };
    let forwarded_arguments = if name == "aperture_context_plan" {
        with_plan_session_hint(&arguments, plan_op, plan_session_affinity.as_deref())
    } else {
        arguments.clone()
    };

    match call_proxy(base_url, api_path, &forwarded_arguments) {
        Ok(result) => {
            let content = result.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let is_error = result
                .get("is_error")
                .and_then(|e| e.as_bool())
                .unwrap_or(false);

            if name == "aperture_context_plan" {
                update_plan_session_affinity(plan_session_affinity, plan_op, is_error, &result);
            }

            send_response(
                id,
                json!({
                    "content": [{"type": "text", "text": content}],
                    "isError": is_error
                }),
            );
        }
        Err(e) => {
            send_response(
                id,
                json!({
                    "content": [{"type": "text", "text": format!("Aperture proxy error: {e}")}],
                    "isError": true
                }),
            );
        }
    }
}

pub fn run_stdio_server() {
    let port = std::env::var("APERTURE_PORT").unwrap_or_else(|_| "5400".into());
    let base_url = format!("http://127.0.0.1:{port}");
    let mut plan_session_affinity: Option<String> = None;

    eprintln!("aperture-mcp: starting (proxy at {base_url})");

    let stdin = io::stdin().lock();
    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("aperture-mcp: stdin read error: {e}");
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("aperture-mcp: invalid JSON: {e}");
                // JSON-RPC parse error
                send_error(&Value::Null, -32700, &format!("Parse error: {e}"));
                continue;
            }
        };

        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        match method {
            "initialize" => {
                eprintln!("aperture-mcp: initialize");
                send_response(
                    &id,
                    json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": "aperture",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }),
                );
            }
            "notifications/initialized" => {
                eprintln!("aperture-mcp: initialized");
                // Notification - no response
            }
            "tools/list" => {
                eprintln!("aperture-mcp: tools/list");
                send_response(
                    &id,
                    json!({
                        "tools": tool_definitions()
                    }),
                );
            }
            "tools/call" => {
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                eprintln!("aperture-mcp: tools/call {tool_name}");
                handle_tools_call(&id, &params, &base_url, &mut plan_session_affinity);
            }
            "ping" => {
                send_response(&id, json!({}));
            }
            "" if id.is_null() => {
                // Notification with no method - ignore
            }
            _ => {
                eprintln!("aperture-mcp: unknown method: {method}");
                if !id.is_null() {
                    send_error(&id, -32601, &format!("Method not found: {method}"));
                }
            }
        }
    }

    eprintln!("aperture-mcp: shutting down");
}
