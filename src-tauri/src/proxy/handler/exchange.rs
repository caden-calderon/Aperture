use tracing::{error, warn};

use crate::proxy::{capture::CapturedExchange, ProxyState};

/// Dispatch events and feed engine after an exchange completes.
///
/// Engine ingest runs on a blocking thread pool via `spawn_blocking` to
/// avoid starving the tokio worker pool during heavy block processing
/// (classification, SQLite persistence). This prevents `/_aperture/` MCP
/// requests from timing out when an ingest is in progress.
pub(super) async fn finalize_exchange(
    state: &ProxyState,
    request_id: &str,
    status: u16,
    exchange: &CapturedExchange,
) {
    // HTTP-level event — fires regardless of whether the engine accepts the ingest.
    if let Some(ref dispatcher) = state.dispatcher {
        let total_tokens = exchange
            .usage
            .as_ref()
            .map(|u| u.input_tokens + u.output_tokens);
        dispatcher.response_complete(request_id, status, total_tokens);
    }

    // Feed engine on a blocking thread to avoid starving the tokio worker pool.
    let applied = if let Some(ref engine) = state.engine {
        let engine = engine.clone();
        let provider = exchange.provider.to_string();
        let model = exchange.model.clone();
        let thread_identity = exchange.thread_identity.clone();
        let request_blocks = exchange.request_blocks.clone();
        let response_blocks = exchange.response_blocks.clone();
        let overhead_tokens = exchange.overhead_tokens;
        let req_block_count = request_blocks.len();
        let resp_block_count = response_blocks.len();
        let request_id_owned = request_id.to_string();

        warn!(
            request_id = %request_id,
            req_blocks = req_block_count,
            resp_blocks = resp_block_count,
            "DIAG: engine.ingest() starting on spawn_blocking"
        );

        let ingest_start = std::time::Instant::now();
        match tokio::task::spawn_blocking(move || {
            engine.ingest(
                &provider,
                &model,
                "proxy",
                thread_identity.as_deref(),
                request_blocks,
                response_blocks,
                overhead_tokens,
            )
        })
        .await
        {
            Ok(result) => {
                warn!(
                    request_id = %request_id_owned,
                    elapsed_ms = ingest_start.elapsed().as_millis() as u64,
                    applied = result.applied,
                    "DIAG: engine.ingest() completed"
                );
                result.applied
            }
            Err(e) => {
                error!(
                    request_id = %request_id_owned,
                    elapsed_ms = ingest_start.elapsed().as_millis() as u64,
                    error = %e,
                    "DIAG: engine.ingest() task FAILED (panic or cancellation)"
                );
                true // fail-open: emit events anyway
            }
        }
    } else {
        true // No engine -> treat as applied for event purposes
    };

    if applied {
        if let Some(ref dispatcher) = state.dispatcher {
            dispatcher.blocks_captured(exchange);
        }
    }
}
