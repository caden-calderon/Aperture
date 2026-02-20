use crate::proxy::{capture::CapturedExchange, ProxyState};

/// Dispatch events and feed engine after an exchange completes.
pub(super) fn finalize_exchange(
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

    // Feed engine, then gate blocks_captured on whether ingest actually applied.
    let applied = if let Some(ref engine) = state.engine {
        let result = engine.ingest(
            &exchange.provider.to_string(),
            &exchange.model,
            "proxy",
            exchange.thread_identity.as_deref(),
            exchange.request_blocks.clone(),
            exchange.response_blocks.clone(),
            exchange.overhead_tokens,
        );
        result.applied
    } else {
        true // No engine -> treat as applied for event purposes
    };

    if applied {
        if let Some(ref dispatcher) = state.dispatcher {
            dispatcher.blocks_captured(exchange);
        }
    }
}
