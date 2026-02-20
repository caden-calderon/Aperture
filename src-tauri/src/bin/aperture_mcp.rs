//! Standalone MCP server binary entrypoint for Aperture context tools.
//!
//! The runtime implementation lives in `aperture_lib::mcp::server` so
//! orchestration logic and tests stay out of this hot binary entrypoint.

fn main() {
    aperture_lib::mcp::server::run_stdio_server();
}
