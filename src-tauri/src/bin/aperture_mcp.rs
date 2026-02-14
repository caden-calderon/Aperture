//! Standalone MCP server for Aperture context tools.
//!
//! Communicates with Claude Code over stdio (MCP protocol: JSON-RPC 2.0,
//! newline-delimited) and translates tool calls into HTTP requests to the
//! Aperture proxy's internal API at `/_aperture/context/*`.
//!
//! Configuration:
//!   APERTURE_PORT — proxy port (default: 5400)
//!
//! Usage in Claude Code MCP settings:
//! ```json
//! {
//!   "mcpServers": {
//!     "aperture": {
//!       "command": "aperture-mcp",
//!       "env": { "APERTURE_PORT": "5400" }
//!     }
//!   }
//! }
//! ```

use std::io::{self, BufRead, Write};

use aperture_lib::metacog::context_tool_definitions;
use serde_json::{json, Value};

/// Map tool names to their HTTP API path suffix.
fn tool_api_path(name: &str) -> Option<&'static str> {
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
fn tool_definitions() -> Value {
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
fn call_proxy(base_url: &str, api_path: &str, arguments: &Value) -> Result<Value, String> {
    let url = format!("{base_url}/_aperture{api_path}");
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(&url)
        .json(arguments)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .map_err(|e| format!("HTTP request to proxy failed: {e}"))?;

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

    Ok(body)
}

/// Handle a tools/call request: forward to proxy, return MCP-format result.
fn handle_tools_call(id: &Value, params: &Value, base_url: &str) {
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

    match call_proxy(base_url, api_path, &arguments) {
        Ok(result) => {
            let content = result.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let is_error = result
                .get("is_error")
                .and_then(|e| e.as_bool())
                .unwrap_or(false);

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

fn main() {
    let port = std::env::var("APERTURE_PORT").unwrap_or_else(|_| "5400".into());
    let base_url = format!("http://127.0.0.1:{port}");

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
                // Notification — no response
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
                handle_tools_call(&id, &params, &base_url);
            }
            "ping" => {
                send_response(&id, json!({}));
            }
            "" if id.is_null() => {
                // Notification with no method — ignore
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_api_path_known_tools() {
        assert_eq!(
            tool_api_path("aperture_context_preview"),
            Some("/context/preview")
        );
        assert_eq!(
            tool_api_path("aperture_context_read"),
            Some("/context/read")
        );
        assert_eq!(
            tool_api_path("aperture_context_search"),
            Some("/context/search")
        );
        assert_eq!(
            tool_api_path("aperture_context_plan"),
            Some("/context/plan")
        );
        assert_eq!(
            tool_api_path("aperture_context_status"),
            Some("/context/status")
        );
    }

    #[test]
    fn test_tool_api_path_unknown() {
        assert_eq!(tool_api_path("unknown_tool"), None);
        assert_eq!(tool_api_path(""), None);
    }

    #[test]
    fn test_tool_definitions_count() {
        let defs = tool_definitions();
        let arr = defs.as_array().unwrap();
        assert_eq!(arr.len(), 5);
    }

    #[test]
    fn test_tool_definitions_have_required_fields() {
        let defs = tool_definitions();
        for tool in defs.as_array().unwrap() {
            assert!(tool.get("name").is_some(), "tool missing name");
            assert!(
                tool.get("description").is_some(),
                "tool missing description"
            );
            let schema = tool.get("inputSchema").expect("tool missing inputSchema");
            assert_eq!(schema["type"], "object");
        }
    }

    #[test]
    fn test_tool_definitions_names_match_api_paths() {
        let defs = tool_definitions();
        for tool in defs.as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            assert!(
                tool_api_path(name).is_some(),
                "tool '{name}' has no API path mapping"
            );
        }
    }

    #[test]
    fn test_tool_definitions_include_plan_split_schema() {
        let defs = tool_definitions();
        let plan_tool = defs
            .as_array()
            .and_then(|tools| {
                tools.iter().find(|tool| {
                    tool.get("name").and_then(|v| v.as_str()) == Some("aperture_context_plan")
                })
            })
            .expect("plan tool should exist");

        assert!(
            plan_tool
                .get("inputSchema")
                .and_then(|schema| schema.get("properties"))
                .and_then(|props| props.get("split"))
                .is_some(),
            "plan schema should include split instructions"
        );
    }
}
