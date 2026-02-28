//! Aperture - Universal LLM context visualization, management, and transparent control proxy.
//!
//! Aperture is a transparent proxy that sits between AI coding tools (Claude Code,
//! Codex, OpenCode, etc.) and their upstream APIs. It captures and visualizes
//! context without requiring any API keys of its own — the tools' existing
//! credentials pass through transparently.
//!
//! The proxy runs as a standalone process (`aperture-proxy`) that survives
//! Tauri/WebKit crashes. This Tauri app is a pure frontend that connects
//! to the proxy via HTTP/SSE.

pub mod build_info;
pub mod engine;
pub mod events;
pub mod mcp;
pub mod metacog;
pub mod proxy;
pub mod terminal;
pub mod util;

use std::time::Duration;

use tracing::{error, info, warn};

// ── App Entry ────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    util::install_crash_diagnostics();
    util::load_env();
    util::init_logging();

    let port = util::get_proxy_port();

    info!("{}", build_info::fingerprint());
    info!("Transparent proxy mode — tools' API keys pass through, no Aperture key needed");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(terminal::TerminalState::new())
        .setup(move |_app| {
            // Ensure the standalone proxy process is running.
            // If not, spawn it as a detached process that survives Tauri exit.
            if !is_proxy_healthy(port) {
                info!("Proxy not running — spawning aperture-proxy process");
                if let Err(e) = spawn_proxy_process() {
                    error!("Failed to spawn proxy process: {e}");
                    // Non-fatal: proxy may already be running or user can start manually
                }

                // Wait for proxy to become healthy
                if !wait_for_proxy(port, Duration::from_secs(5)) {
                    warn!(
                        "Proxy did not become healthy within 5s. \
                         UI will connect when proxy starts. \
                         Run `cargo run --bin aperture-proxy` manually if needed."
                    );
                }
            } else {
                info!("Proxy already running on port {port}");
            }

            info!("Proxy address: http://127.0.0.1:{port}");
            info!("Usage: ANTHROPIC_BASE_URL=http://localhost:{port} claude", );

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Terminal (PTY is process-local, stays as Tauri IPC)
            terminal::spawn_shell,
            terminal::spawn_provider_shell,
            terminal::start_codex_subscription_bridge,
            terminal::stop_codex_subscription_bridge,
            terminal::is_codex_subscription_bridge_running,
            terminal::send_input,
            terminal::resize_terminal,
            terminal::kill_session,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Simple event loop — no crash prevention needed since proxy is independent.
    app.run(|_app_handle, _event| {});
}

/// Check if the proxy is healthy by hitting the health endpoint.
fn is_proxy_healthy(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
        return false;
    }

    // TCP connected — verify it's actually aperture via HTTP health check
    let url = format!("http://127.0.0.1:{port}/_aperture/health");
    reqwest::blocking::Client::new()
        .get(&url)
        .timeout(Duration::from_millis(500))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Spawn the `aperture-proxy` binary as a detached process.
fn spawn_proxy_process() -> Result<(), String> {
    // Look for the binary next to the current executable first,
    // then fall back to PATH lookup (for development with `cargo run`).
    let binary = find_proxy_binary();

    info!("Spawning proxy: {:?}", binary);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        // Direct proxy stderr to a log file so diagnostics survive Tauri exit.
        // When run manually, proxy still logs to terminal (its own stderr default).
        let log_path = std::path::Path::new("/tmp/aperture-proxy.log");
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|e| format!("failed to open proxy log {}: {e}", log_path.display()))?;

        // Use setsid via pre_exec to create a new session, ensuring the
        // child process survives when the parent (Tauri) exits.
        let mut cmd = Command::new(&binary);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log_file));

        // SAFETY: pre_exec runs between fork and exec. setsid() is
        // async-signal-safe and creates a new process group + session.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        cmd.spawn()
            .map_err(|e| format!("failed to spawn {}: {e}", binary.display()))?;
    }

    #[cfg(not(unix))]
    {
        use std::process::{Command, Stdio};
        Command::new(&binary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("failed to spawn {}: {e}", binary.display()))?;
    }

    Ok(())
}

/// Find the aperture-proxy binary. Checks next to the current executable
/// first (production), then tries PATH (development).
fn find_proxy_binary() -> std::path::PathBuf {
    // Check next to current executable (Tauri bundle)
    if let Ok(current_exe) = std::env::current_exe() {
        let sibling = current_exe.parent().unwrap().join("aperture-proxy");
        if sibling.exists() {
            return sibling;
        }
    }

    // Check Cargo target directory (development)
    let target_debug = std::path::Path::new("target/debug/aperture-proxy");
    if target_debug.exists() {
        return target_debug.to_path_buf();
    }

    // Fallback: assume it's in PATH
    std::path::PathBuf::from("aperture-proxy")
}

/// Poll the proxy health endpoint until it responds or timeout.
fn wait_for_proxy(port: u16, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(100);

    while start.elapsed() < timeout {
        if is_proxy_healthy(port) {
            info!("Proxy is healthy after {:?}", start.elapsed());
            return true;
        }
        std::thread::sleep(poll_interval);
    }

    false
}
