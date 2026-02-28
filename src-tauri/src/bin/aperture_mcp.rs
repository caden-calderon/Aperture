//! Standalone MCP server binary entrypoint for Aperture context tools.
//!
//! The runtime implementation lives in `aperture_lib::mcp::server` so
//! orchestration logic and tests stay out of this hot binary entrypoint.

use aperture_lib::build_info;

fn main() {
    // --version flag: print fingerprint to stdout and exit.
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{}", build_info::fingerprint());
        return;
    }

    // MCP uses stdio — diagnostics go to stderr only.
    eprintln!("{}", build_info::fingerprint());

    aperture_lib::mcp::server::run_stdio_server();
}
