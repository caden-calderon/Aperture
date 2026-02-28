//! Standalone proxy process for Aperture.
//!
//! Runs the proxy + context engine as an independent OS process that survives
//! Tauri/WebKit crashes. The Tauri app connects to it via HTTP/SSE.

use std::sync::Arc;

use aperture_lib::build_info;
use aperture_lib::engine::ContextEngine;
use aperture_lib::events::broadcaster::BroadcastDispatcher;
use aperture_lib::proxy;
use aperture_lib::util;

#[tokio::main]
async fn main() {
    // --version flag: print fingerprint and exit (before any init).
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{}", build_info::fingerprint());
        return;
    }

    util::install_crash_diagnostics();
    util::load_env();
    util::init_logging();

    let port = util::get_proxy_port();

    tracing::info!("{}", build_info::fingerprint());
    tracing::info!("Starting standalone proxy on port {port}");

    // Create broadcast dispatcher for SSE event delivery
    let broadcaster = Arc::new(BroadcastDispatcher::new());
    let dyn_dispatcher = broadcaster.to_dyn_dispatcher();

    // Create context engine with the broadcast dispatcher
    let engine = Arc::new(ContextEngine::new(Some(dyn_dispatcher)));

    if let Err(e) = proxy::start_proxy(port, None, None, Some(engine), Some(broadcaster)).await {
        tracing::error!("Proxy server error: {e}");
        std::process::exit(1);
    }
}
