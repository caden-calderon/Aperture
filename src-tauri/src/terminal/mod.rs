mod codex_bridge;
mod error;
mod session;

use std::collections::HashMap;
use std::sync::Mutex;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tauri::{AppHandle, State};
use tracing::{debug, info};
use uuid::Uuid;

use error::TerminalError;
use session::TerminalSession;

/// Default proxy port — must match proxy::DEFAULT_PORT.
const PROXY_PORT: u16 = 5400;
const OPENAI_PROXY_ENV_KEYS: [&str; 2] = ["OPENAI_BASE_URL", "OPENAI_API_BASE"];

#[derive(Debug, Default)]
struct EnvPlan {
    set: HashMap<String, String>,
    clear: Vec<String>,
}

pub struct TerminalState {
    sessions: Mutex<HashMap<String, TerminalSession>>,
    codex_bridge: Mutex<Option<codex_bridge::CodexBridgeHandle>>,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            codex_bridge: Mutex::new(None),
        }
    }
}

impl TerminalState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Detect user's shell from $SHELL or fall back to /bin/sh.
fn detect_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Build env settings for a provider launch mode.
fn provider_env_plan(provider: &str, port: u16) -> EnvPlan {
    let base = format!("http://localhost:{port}");
    let mut env = HashMap::new();
    let mut clear = Vec::new();

    match provider {
        "anthropic" => {
            env.insert("ANTHROPIC_BASE_URL".into(), base);
            clear.extend(OPENAI_PROXY_ENV_KEYS.iter().map(|k| (*k).to_string()));
        }
        // Direct ChatGPT/Codex mode: do not force OpenAI API base.
        // Also clear inherited proxy vars so this mode is deterministic.
        "openai" => {
            env.insert("ANTHROPIC_BASE_URL".into(), base);
            clear.extend(OPENAI_PROXY_ENV_KEYS.iter().map(|k| (*k).to_string()));
        }
        "openai_proxy" => {
            // Explicit API/proxy mode for OpenAI-compatible tools.
            env.insert("ANTHROPIC_BASE_URL".into(), base.clone());
            env.insert("OPENAI_BASE_URL".into(), base.clone());
            env.insert("OPENAI_API_BASE".into(), base);
        }
        _ => {}
    }

    EnvPlan { set: env, clear }
}

/// Build default proxy env for interactive shells so manual `claude`/`codex`
/// commands typed by the user are routed through Aperture.
fn default_proxy_env_plan(port: u16) -> EnvPlan {
    // Keep Claude proxied by default. Do NOT force OPENAI_BASE_URL globally,
    // otherwise Codex ChatGPT subscription mode is coerced into API auth mode.
    provider_env_plan("anthropic", port)
}

/// Spawn a PTY shell, optionally injecting extra environment variables.
fn spawn_pty(
    app: AppHandle,
    state: &TerminalState,
    cols: Option<u16>,
    rows: Option<u16>,
    extra_env: HashMap<String, String>,
    clear_env: Vec<String>,
    command: Option<String>,
) -> Result<String, TerminalError> {
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows: rows.unwrap_or(24),
            cols: cols.unwrap_or(80),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

    let shell = detect_shell();
    info!("Spawning shell: {shell}");

    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");

    // Inherit HOME so shell config loads
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", &home);
    }

    // Clear env vars first to avoid inherited values forcing wrong auth mode.
    for key in &clear_env {
        cmd.env_remove(key);
        debug!("Clearing env: {key}");
    }

    // Inject provider / proxy env vars
    for (key, val) in &extra_env {
        cmd.env(key, val);
        debug!("Injecting env: {key}={val}");
    }

    // If a command was requested, pass it with -c so the shell executes it
    if let Some(ref run_cmd) = command {
        cmd.args(["-c", run_cmd]);
        info!("Running command in shell: {run_cmd}");
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

    let session_id = Uuid::new_v4().to_string();
    let session = TerminalSession::new(session_id.clone(), child, writer, pair.master, app)?;

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;
    sessions.insert(session_id.clone(), session);

    debug!("Terminal session created: {session_id}");
    Ok(session_id)
}

/// Spawn a plain shell (backwards compatible — no extra env).
#[tauri::command]
pub fn spawn_shell(
    app: AppHandle,
    state: State<'_, TerminalState>,
    cols: Option<u16>,
    rows: Option<u16>,
    env_vars: Option<HashMap<String, String>>,
) -> Result<String, TerminalError> {
    let port = std::env::var("APERTURE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(PROXY_PORT);

    let env_plan = match env_vars {
        Some(set) => EnvPlan {
            set,
            clear: Vec::new(),
        },
        None => default_proxy_env_plan(port),
    };

    spawn_pty(app, &state, cols, rows, env_plan.set, env_plan.clear, None)
}

/// Spawn a shell pre-configured for a specific LLM provider.
///
/// Injects the correct `*_BASE_URL` env vars so the provider CLI
/// routes traffic through Aperture's proxy automatically.
#[tauri::command]
pub fn spawn_provider_shell(
    app: AppHandle,
    state: State<'_, TerminalState>,
    cols: Option<u16>,
    rows: Option<u16>,
    provider: String,
    command: Option<String>,
) -> Result<String, TerminalError> {
    let port = std::env::var("APERTURE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(PROXY_PORT);

    let env_plan = provider_env_plan(&provider, port);
    spawn_pty(
        app,
        &state,
        cols,
        rows,
        env_plan.set,
        env_plan.clear,
        command,
    )
}

#[tauri::command]
pub fn send_input(
    state: State<'_, TerminalState>,
    session_id: String,
    data: String,
) -> Result<(), TerminalError> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| TerminalError::WriteFailed(e.to_string()))?;

    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| TerminalError::SessionNotFound(session_id.clone()))?;

    session.write(&data).map_err(TerminalError::WriteFailed)
}

#[tauri::command]
pub fn resize_terminal(
    state: State<'_, TerminalState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), TerminalError> {
    if cols == 0 || rows == 0 || cols > 500 || rows > 500 {
        return Err(TerminalError::ResizeFailed(format!(
            "Invalid dimensions: {cols}x{rows} (must be 1-500)"
        )));
    }

    let sessions = state
        .sessions
        .lock()
        .map_err(|e| TerminalError::ResizeFailed(e.to_string()))?;

    let session = sessions
        .get(&session_id)
        .ok_or_else(|| TerminalError::SessionNotFound(session_id.clone()))?;

    session
        .resize(cols, rows)
        .map_err(TerminalError::ResizeFailed)?;

    debug!("Resized {session_id}: {cols}x{rows}");
    Ok(())
}

#[tauri::command]
pub fn kill_session(
    state: State<'_, TerminalState>,
    session_id: String,
) -> Result<(), TerminalError> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|e| TerminalError::SessionNotFound(e.to_string()))?;

    if let Some(mut session) = sessions.remove(&session_id) {
        session.kill();
        debug!("Terminal session killed: {session_id}");
        Ok(())
    } else {
        Err(TerminalError::SessionNotFound(session_id))
    }
}

#[tauri::command]
pub fn start_codex_subscription_bridge(
    app: AppHandle,
    state: State<'_, TerminalState>,
) -> Result<bool, TerminalError> {
    let mut guard = state
        .codex_bridge
        .lock()
        .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

    if guard.is_some() {
        return Ok(false);
    }

    let handle =
        codex_bridge::spawn(app).map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;
    *guard = Some(handle);
    Ok(true)
}

#[tauri::command]
pub fn stop_codex_subscription_bridge(state: State<'_, TerminalState>) -> Result<bool, TerminalError> {
    let mut guard = state
        .codex_bridge
        .lock()
        .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

    if let Some(handle) = guard.take() {
        handle.stop();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub fn is_codex_subscription_bridge_running(
    state: State<'_, TerminalState>,
) -> Result<bool, TerminalError> {
    let guard = state
        .codex_bridge
        .lock()
        .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;
    Ok(guard.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_env_vars_anthropic() {
        let env = provider_env_plan("anthropic", 5400);
        assert_eq!(
            env.set.get("ANTHROPIC_BASE_URL").unwrap(),
            "http://localhost:5400"
        );
        assert!(!env.set.contains_key("OPENAI_BASE_URL"));
        assert_eq!(
            env.clear,
            vec!["OPENAI_BASE_URL".to_string(), "OPENAI_API_BASE".to_string()]
        );
    }

    #[test]
    fn test_provider_env_vars_openai() {
        let env = provider_env_plan("openai", 5400);
        assert!(!env.set.contains_key("OPENAI_BASE_URL"));
        assert_eq!(
            env.set.get("ANTHROPIC_BASE_URL").unwrap(),
            "http://localhost:5400"
        );
        assert_eq!(
            env.clear,
            vec!["OPENAI_BASE_URL".to_string(), "OPENAI_API_BASE".to_string()]
        );
    }

    #[test]
    fn test_provider_env_vars_openai_proxy() {
        let env = provider_env_plan("openai_proxy", 5400);
        assert_eq!(
            env.set.get("OPENAI_BASE_URL").unwrap(),
            "http://localhost:5400"
        );
        assert_eq!(
            env.set.get("OPENAI_API_BASE").unwrap(),
            "http://localhost:5400"
        );
        assert_eq!(
            env.set.get("ANTHROPIC_BASE_URL").unwrap(),
            "http://localhost:5400"
        );
        assert!(env.clear.is_empty());
    }

    #[test]
    fn test_provider_env_vars_custom_port() {
        let env = provider_env_plan("anthropic", 8080);
        assert_eq!(
            env.set.get("ANTHROPIC_BASE_URL").unwrap(),
            "http://localhost:8080"
        );
    }

    #[test]
    fn test_provider_env_vars_unknown_returns_empty() {
        let env = provider_env_plan("gemini", 5400);
        assert!(env.set.is_empty());
        assert!(env.clear.is_empty());
    }

    #[test]
    fn test_default_proxy_env_vars_includes_both_providers() {
        let env = default_proxy_env_plan(5400);
        assert_eq!(
            env.set.get("ANTHROPIC_BASE_URL").unwrap(),
            "http://localhost:5400"
        );
        assert!(!env.set.contains_key("OPENAI_BASE_URL"));
        assert_eq!(
            env.clear,
            vec!["OPENAI_BASE_URL".to_string(), "OPENAI_API_BASE".to_string()]
        );
    }

    #[test]
    fn test_detect_shell_returns_string() {
        let shell = detect_shell();
        assert!(!shell.is_empty());
    }
}
