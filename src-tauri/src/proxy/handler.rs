//! Request handler for the proxy.

mod exchange;
mod headers;
mod routing;
#[cfg(test)]
mod tests;

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, instrument, warn, Level};
use uuid::Uuid;

use self::exchange::finalize_exchange;
use self::headers::{
    connection_header_tokens, convert_headers, has_zstd_content_encoding, log_headers,
    should_strip_request_header,
};
use self::routing::{build_upstream_url, determine_upstream, is_supported_api_path};
use super::{error::ProxyError, hot_patch, ProxyState, MAX_BODY_SIZE};

/// Main proxy handler for all requests.
#[instrument(skip_all, fields(request_id = %Uuid::new_v4()))]
pub async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let request_id = Uuid::new_v4().to_string();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();

    info!("--> {} {}", method, path);
    log_headers("Request", req.headers());

    // Internal API routes — handled locally, never forwarded upstream.
    if path.starts_with("/_aperture/") || path == "/_aperture" {
        return super::context_api::handle_aperture_route(&state, &path, req)
            .await
            .unwrap_or_else(|| (StatusCode::NOT_FOUND, "Unknown aperture route").into_response())
            .into_response();
    }

    let upstream = determine_upstream(&state.config, req.headers(), &path);
    let upstream_url = build_upstream_url(upstream.url, &path, uri.query(), upstream.is_chatgpt);

    debug!("Forwarding to: {}", upstream_url);

    match forward_request(
        &state,
        &request_id,
        method.as_ref(),
        &path,
        req,
        &upstream_url,
    )
    .await
    {
        Ok(response) => {
            let status = response.status();
            info!("<-- {} {} -> {}", method, path, status);
            response
        }
        Err(e) => {
            error!("Proxy error: {}", e);

            // Emit error event
            if let Some(ref dispatcher) = state.dispatcher {
                dispatcher.proxy_error(Some(&request_id), &e.to_string());
            }
            state.capture.mark_failed(&request_id);

            let status = match &e {
                ProxyError::RequestTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
                ProxyError::UpstreamTimeout => StatusCode::GATEWAY_TIMEOUT,
                _ => StatusCode::BAD_GATEWAY,
            };
            (status, format!("Proxy error: {e}")).into_response()
        }
    }
}

/// Handle a streaming SSE response: tee chunks to client + accumulate for capture.
#[allow(clippy::too_many_arguments)]
fn handle_streaming_response(
    state: Arc<ProxyState>,
    request_id: String,
    upstream_url: String,
    status: StatusCode,
    headers: reqwest::header::HeaderMap,
    upstream_response: reqwest::Response,
    request_start: Instant,
    upstream_latency: Duration,
) -> Response {
    let mut upstream_stream = upstream_response.bytes_stream();
    let upstream_latency_ms = upstream_latency.as_millis() as u64;
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    tokio::spawn(async move {
        let mut total_bytes: u64 = 0;
        let mut chunk_count: u64 = 0;
        let mut stream_error: Option<String> = None;

        while let Some(chunk_result) = upstream_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    chunk_count += 1;
                    let bytes_received = state.capture.append_sse_chunk(&request_id, &chunk);
                    total_bytes = bytes_received;

                    // Emit streaming progress (throttled — every 4KB)
                    if let Some(ref dispatcher) = state.dispatcher {
                        if bytes_received % 4096 < chunk.len() as u64 {
                            dispatcher.response_streaming(&request_id, bytes_received);
                        }
                    }

                    if tx.send(Ok(chunk)).await.is_err() {
                        debug!(request_id = %request_id, "Client disconnected during SSE stream");
                        break;
                    }
                }
                Err(e) => {
                    stream_error = Some(e.to_string());
                    let _ = tx.send(Err(std::io::Error::other(e.to_string()))).await;
                    break;
                }
            }
        }

        // Log stream completion diagnostics
        if let Some(ref err) = stream_error {
            error!(
                request_id = %request_id,
                upstream = %upstream_url,
                bytes_received = total_bytes,
                chunks = chunk_count,
                elapsed_ms = request_start.elapsed().as_millis() as u64,
                error = %err,
                "SSE stream disconnected with error"
            );
        } else if total_bytes == 0 {
            error!(
                request_id = %request_id,
                upstream = %upstream_url,
                elapsed_ms = request_start.elapsed().as_millis() as u64,
                "SSE stream completed with zero bytes (possible upstream rejection)"
            );
        }

        // Finalize capture and dispatch events
        if let Some(exchange) = state.capture.finalize_streaming(&request_id) {
            warn!(
                request_id = %request_id,
                bytes = total_bytes,
                chunks = chunk_count,
                "DIAG: SSE stream complete, starting finalize_exchange"
            );

            finalize_exchange(&state, &request_id, status.as_u16(), &exchange).await;

            debug!(
                request_id = %request_id,
                upstream_latency_ms,
                total_latency_ms = request_start.elapsed().as_millis() as u64,
                bytes_received = exchange.bytes_received,
                response_status = status.as_u16(),
                "Completed streaming proxy request"
            );
        }
    });

    let body = Body::from_stream(ReceiverStream::new(rx));
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = convert_headers(&headers);
    response
}

/// Handle a non-streaming response: read body, capture, emit events.
///
/// If context tool calls are detected and the engine is available, attempts
/// interception (dispatch + optional re-invoke) before returning to the client.
#[allow(clippy::too_many_arguments)]
async fn handle_non_streaming_response(
    state: &Arc<ProxyState>,
    request_id: &str,
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    upstream_response: reqwest::Response,
    request_was_captured: bool,
    request_start: Instant,
    upstream_latency: Duration,
    parsed: Option<&super::parser::ParsedRequest>,
    original_request_body: &[u8],
    upstream_url: &str,
    original_request_headers: &HeaderMap,
) -> Result<Response, ProxyError> {
    let response_bytes = upstream_response.bytes().await?;

    // Log response body preview (slice raw bytes BEFORE lossy conversion
    // to avoid panicking on multi-byte char boundaries).
    if tracing::enabled!(Level::DEBUG) {
        let preview = if response_bytes.len() > 500 {
            format!(
                "{}... ({} bytes total)",
                String::from_utf8_lossy(&response_bytes[..500]),
                response_bytes.len()
            )
        } else {
            String::from_utf8_lossy(&response_bytes).to_string()
        };
        debug!("Response body: {}", preview);
    }

    // Attempt context tool interception on successful responses
    if status.is_success() {
        if let (Some(parsed), Some(ref engine)) = (parsed, &state.engine) {
            if !parsed.stream {
                let uri_path = upstream_url
                    .find("://")
                    .and_then(|i| upstream_url[i + 3..].find('/'))
                    .map(|i| &upstream_url[upstream_url.find("://").unwrap() + 3 + i..])
                    .unwrap_or("/");

                if let Some(result) = super::interceptor::try_context_tool_interception(
                    state,
                    request_id,
                    uri_path,
                    parsed,
                    engine,
                    &response_bytes,
                    original_request_body,
                    upstream_url,
                    original_request_headers,
                    request_start,
                )
                .await
                {
                    let super::interceptor::InterceptionResult::ModifiedResponse(body) = result;

                    // Capture the effective response body the client receives.
                    if request_was_captured {
                        if let Some(exchange) =
                            state
                                .capture
                                .capture_response(request_id, status.as_u16(), &body)
                        {
                            finalize_exchange(state, request_id, status.as_u16(), &exchange).await;
                        }
                    }

                    debug!(
                        request_id = %request_id,
                        "Returning intercepted response ({} bytes)",
                        body.len()
                    );

                    let mut response = Response::new(Body::from(body));
                    *response.status_mut() = status;
                    *response.headers_mut() = convert_headers(headers);
                    return Ok(response);
                }
            }
        }
    }

    if request_was_captured {
        if let Some(exchange) =
            state
                .capture
                .capture_response(request_id, status.as_u16(), &response_bytes)
        {
            finalize_exchange(state, request_id, status.as_u16(), &exchange).await;
        }
    }

    debug!(
        request_id = %request_id,
        upstream_latency_ms = upstream_latency.as_millis() as u64,
        total_latency_ms = request_start.elapsed().as_millis() as u64,
        response_bytes = response_bytes.len(),
        response_status = status.as_u16(),
        "Completed non-streaming proxy request"
    );

    let mut response = Response::new(Body::from(response_bytes));
    *response.status_mut() = status;
    *response.headers_mut() = convert_headers(headers);
    Ok(response)
}

/// Forward a request to the upstream server with capture integration.
async fn forward_request(
    state: &Arc<ProxyState>,
    request_id: &str,
    method: &str,
    path: &str,
    req: Request<Body>,
    upstream_url: &str,
) -> Result<Response, ProxyError> {
    let request_start = Instant::now();
    let pre_forward_start = Instant::now();
    let (parts, body) = req.into_parts();
    let capture_supported = is_supported_api_path(path);

    // Read body for capture and forwarding
    let body_bytes = axum::body::to_bytes(body, MAX_BODY_SIZE)
        .await
        .map_err(|e| {
            if e.to_string().contains("length limit") {
                ProxyError::RequestTooLarge {
                    size: 0,
                    limit: MAX_BODY_SIZE,
                }
            } else {
                ProxyError::InvalidRequest(e.to_string())
            }
        })?;

    // Detect and decompress zstd content-encoding.
    // Codex CLI sends zstd-compressed request bodies — we need to decompress
    // for hot-patch matching and capture JSON parsing, but forward the original
    // compressed bytes when no patches are applied (transparent byte-passthrough).
    let is_zstd = has_zstd_content_encoding(&parts.headers);

    let decompressed_body = if is_zstd && capture_supported {
        match zstd::stream::decode_all(std::io::Cursor::new(&body_bytes)) {
            Ok(decoded) => {
                debug!(
                    "Decompressed zstd request body: {} -> {} bytes",
                    body_bytes.len(),
                    decoded.len()
                );
                Some(Bytes::from(decoded))
            }
            Err(e) => {
                debug!(
                    "Failed to decompress zstd body ({e}); forwarding original bytes and skipping JSON transform"
                );
                None
            }
        }
    } else {
        None
    };

    let body_for_processing = decompressed_body.as_ref().unwrap_or(&body_bytes);

    if tracing::enabled!(Level::DEBUG) && !body_for_processing.is_empty() {
        let preview = if body_for_processing.len() > 500 {
            format!(
                "{}... ({} bytes total)",
                String::from_utf8_lossy(&body_for_processing[..500]),
                body_for_processing.len()
            )
        } else {
            String::from_utf8_lossy(body_for_processing).to_string()
        };
        debug!("Request body: {}", preview);
    }

    // Apply hot patches before rewriting/capture so downstream parsing sees
    // effective request semantics.
    // peek_all() keeps patches persistent — LLM tools re-send their own
    // conversation history, so patches must re-apply on every request until
    // the user explicitly clears them via clear_hot_patches.
    let pending_patches = state.hot_patches.peek_all();
    let (forwarded_body, body_was_patched) = if !pending_patches.is_empty() {
        match hot_patch::apply_patches(body_for_processing, &pending_patches) {
            Some(patched) => {
                debug!(
                    "Hot patches applied, body modified ({} -> {} bytes)",
                    body_for_processing.len(),
                    patched.len()
                );
                (Bytes::from(patched), true)
            }
            None => (body_bytes.clone(), false),
        }
    } else {
        (body_bytes.clone(), false)
    };

    // Apply context mutations (planner rewriting, tool cleanup, trailing context).
    // Shadows forwarded_body with the rewritten payload if changes were made.
    // Fail-open: if rewriting fails for any reason, forward the original body.
    let rewrite_input = if body_was_patched {
        forwarded_body.as_ref()
    } else {
        body_for_processing
    };
    let parsed_for_rewrite = if capture_supported {
        super::parser::parse_request(path, rewrite_input).ok()
    } else {
        None
    };

    // Parse blocks from the original request (pre-rewrite) for persistent archival.
    // These IDs include content the stateless client re-sent, even if previously archived.
    let pre_rewrite_blocks = parsed_for_rewrite
        .as_ref()
        .map(|p| p.blocks.clone())
        .unwrap_or_default();

    let (forwarded_body, body_was_patched) = if let Some(ref engine) = state.engine {
        if let Some(ref parsed) = parsed_for_rewrite {
            match super::rewriter::rewrite_request(
                rewrite_input,
                path,
                parsed,
                engine,
                &pre_rewrite_blocks,
            ) {
                Ok(Some(rewritten)) => {
                    debug!(
                        "Context rewriting applied ({} -> {} bytes)",
                        rewrite_input.len(),
                        rewritten.len()
                    );
                    (rewritten, true)
                }
                Ok(None) => (forwarded_body, body_was_patched),
                Err(e) => {
                    debug!("Context rewriting skipped, forwarding original: {e}");
                    (forwarded_body, body_was_patched)
                }
            }
        } else {
            (forwarded_body, body_was_patched)
        }
    } else {
        (forwarded_body, body_was_patched)
    };

    // Capture from the effective body semantics (post-rewrite if rewritten).
    let capture_body = if body_was_patched {
        forwarded_body.as_ref()
    } else {
        body_for_processing
    };

    if let Some(alert) = state.runaway_guard.record_proxy_request(capture_body.len()) {
        warn!(
            request_id = %request_id,
            channel = alert.channel,
            count = alert.count,
            threshold = alert.threshold,
            window_secs = alert.window_secs,
            body_bytes = capture_body.len(),
            hard_limit = alert.hard_limit,
            "Runaway guardrail warning: sustained proxy request burst"
        );
        if let Some(ref dispatcher) = state.dispatcher {
            dispatcher.proxy_error(
                Some(request_id),
                &format!(
                    "Guardrail warning: high request burst detected ({} requests in {}s).",
                    alert.count, alert.window_secs
                ),
            );
        }
    }

    let parsed = state
        .capture
        .capture_request(request_id, path, capture_body);

    // Fix H9: When the body was rewritten (archival removed early turns), the
    // capture parsed the POST-REWRITE body which may have a different
    // thread_identity than the PRE-REWRITE body. Override with the PRE-REWRITE
    // identity so ingest (via finalize_exchange) resolves the same session as
    // the rewriter. Without this, plans get committed under session_B (ingest)
    // but the rewriter reads from session_A → plans never fire.
    if parsed.is_some() {
        if let Some(ref pre_rewrite) = parsed_for_rewrite {
            state
                .capture
                .set_thread_identity(request_id, pre_rewrite.thread_identity.clone());
        }
    }

    if let (Some(ref dispatcher), Some(ref parsed)) = (&state.dispatcher, &parsed) {
        dispatcher.request_captured(request_id, method, path, &parsed.provider.to_string());
    }

    // Build upstream request, forwarding headers with hop-by-hop stripping.
    let mut upstream_req = state.client.request(parts.method, upstream_url);
    let connection_tokens = connection_header_tokens(&parts.headers);
    for (key, value) in parts.headers.iter() {
        if should_strip_request_header(key, &connection_tokens, body_was_patched) {
            if body_was_patched && key == header::CONTENT_ENCODING {
                debug!("Stripping Content-Encoding header (body was decompressed for patching)");
            }
            if connection_tokens.contains(key) {
                debug!("Stripping Connection-nominated hop-by-hop header: {}", key);
            }
            if key == header::ACCEPT_ENCODING {
                debug!("Stripping Accept-Encoding request header for byte-stable proxying");
            }
            if key == header::HOST || key == header::CONTENT_LENGTH {
                debug!("Stripping transport-specific request header: {}", key);
            }
            continue;
        }
        upstream_req = upstream_req.header(key, value);
    }

    let pre_forward_overhead = pre_forward_start.elapsed();
    if pre_forward_overhead > Duration::from_millis(5) {
        debug!(
            request_id = %request_id,
            overhead_ms = pre_forward_overhead.as_millis(),
            body_bytes = forwarded_body.len(),
            "Proxy pre-forward overhead exceeded 5ms"
        );
    } else {
        debug!(
            request_id = %request_id,
            overhead_us = pre_forward_overhead.as_micros(),
            body_bytes = forwarded_body.len(),
            "Proxy pre-forward overhead"
        );
    }

    // Send request upstream (keep a copy for potential re-invoke in interceptor)
    let forwarded_body_for_intercept = forwarded_body.clone();
    let upstream_send_start = Instant::now();
    let upstream_response = upstream_req
        .body(forwarded_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ProxyError::UpstreamTimeout
            } else {
                ProxyError::UpstreamFailed(e)
            }
        })?;
    let upstream_latency = upstream_send_start.elapsed();

    let status = upstream_response.status();
    let headers = upstream_response.headers().clone();
    log_headers("Response", &headers);

    let is_streaming = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    if is_streaming {
        debug!("Streaming SSE response");
        Ok(handle_streaming_response(
            state.clone(),
            request_id.to_string(),
            upstream_url.to_string(),
            status,
            headers,
            upstream_response,
            request_start,
            upstream_latency,
        ))
    } else {
        handle_non_streaming_response(
            state,
            request_id,
            status,
            &headers,
            upstream_response,
            parsed.is_some(),
            request_start,
            upstream_latency,
            parsed.as_ref(),
            &forwarded_body_for_intercept,
            upstream_url,
            &parts.headers,
        )
        .await
    }
}
