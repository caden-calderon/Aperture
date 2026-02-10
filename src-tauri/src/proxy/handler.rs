//! Request handler for the proxy.

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
use tracing::{debug, error, info, instrument, Level};
use uuid::Uuid;

use super::{error::ProxyError, hot_patch, ProxyState, UpstreamConfig, MAX_BODY_SIZE};

/// Header value for zstd content encoding.
const CONTENT_ENCODING_ZSTD: &str = "zstd";

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

/// Upstream routing result — carries both the URL and whether this is a
/// ChatGPT Codex backend route (which needs bare paths, no `/v1/` prefix).
struct UpstreamRoute<'a> {
    url: &'a str,
    is_chatgpt: bool,
}

/// Determine which upstream to use based on request characteristics.
///
/// Supports both `/v1/` prefixed and bare paths (e.g. Codex sends `/responses`).
/// Detects ChatGPT subscription tokens (non-`sk-` Bearer) on Responses API paths
/// and routes to the ChatGPT backend instead of the standard OpenAI API.
fn determine_upstream<'a>(config: &'a UpstreamConfig, headers: &HeaderMap, path: &str) -> UpstreamRoute<'a> {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

    let route = |url: &'a str| UpstreamRoute { url, is_chatgpt: false };
    let chatgpt = |url: &'a str| UpstreamRoute { url, is_chatgpt: true };

    // Check for Anthropic-specific header
    if headers.contains_key("x-api-key") || headers.contains_key("anthropic-version") {
        return route(&config.anthropic_url);
    }

    // Check for OpenAI-style authorization (Bearer token)
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                let is_api_key = token.starts_with("sk-");

                // Path-based disambiguation for bearer tokens
                if is_responses_path(path) {
                    // ChatGPT/Codex subscription tokens are NOT `sk-` prefixed.
                    // Route them to chatgpt.com/backend-api/codex which accepts
                    // subscription auth. API keys go to api.openai.com as usual.
                    return if is_api_key {
                        route(&config.openai_url)
                    } else {
                        chatgpt(&config.chatgpt_codex_url)
                    };
                }
                if is_chat_completions_path(path) {
                    return route(&config.openai_url);
                }
                if is_messages_path(path) {
                    return route(&config.anthropic_url);
                }
                // Default bearer to OpenAI
                return route(&config.openai_url);
            }
        }
    }

    // Path-based detection (fallback for requests without auth headers)
    if is_messages_path(path) {
        return route(&config.anthropic_url);
    }
    if is_chat_completions_path(path) || is_responses_path(path) {
        return route(&config.openai_url);
    }

    // Default to Anthropic (primary use case)
    route(&config.anthropic_url)
}

/// Normalize bare API paths by adding the `/v1/` prefix if missing.
///
/// Codex and some tools send requests to `/responses` or `/chat/completions`
/// without the `/v1/` prefix. The standard OpenAI/Anthropic APIs require it,
/// but the ChatGPT Codex backend does NOT — it expects bare paths like
/// `/responses`. The `is_chatgpt` flag comes from `determine_upstream`.
fn normalize_api_path(path: &str, is_chatgpt: bool) -> String {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

    // ChatGPT Codex backend uses bare paths — do NOT add /v1/ prefix.
    if is_chatgpt {
        return path.to_string();
    }

    // Standard APIs: normalize bare paths by adding /v1/ prefix.
    if (is_messages_path(path) || is_chat_completions_path(path) || is_responses_path(path))
        && !path.starts_with("/v1/")
    {
        format!("/v1{path}")
    } else {
        path.to_string()
    }
}

/// Build upstream URL from base + normalized API path, preserving query params.
fn build_upstream_url(
    upstream_base: &str,
    path: &str,
    query: Option<&str>,
    is_chatgpt: bool,
) -> String {
    let normalized_path = normalize_api_path(path, is_chatgpt);
    match query {
        Some(q) if !q.is_empty() => format!("{upstream_base}{normalized_path}?{q}"),
        _ => format!("{upstream_base}{normalized_path}"),
    }
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

    // Detect zstd content-encoding on the request body.
    // Codex CLI sends zstd-compressed request bodies — we need to decompress
    // for hot-patch matching and capture JSON parsing, but forward the original
    // compressed bytes when no patches are applied (transparent byte-passthrough).
    let is_zstd = parts
        .headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|ce| ce.contains(CONTENT_ENCODING_ZSTD))
        .unwrap_or(false);

    // Decompress for internal processing (hot-patch + capture).
    // The original `body_bytes` are preserved for transparent forwarding.
    let decompressed_body = if is_zstd {
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
                debug!("Failed to decompress zstd body ({e}), skipping hot-patch/capture parsing");
                None
            }
        }
    } else {
        None
    };

    // Body for JSON processing: decompressed if zstd, otherwise raw bytes.
    let body_for_processing = decompressed_body.as_ref().unwrap_or(&body_bytes);

    // Log request body preview only when debug logging is enabled.
    // Slice raw bytes BEFORE lossy conversion to avoid panicking on
    // multi-byte char boundaries.
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

    // Apply hot patches BEFORE capture so blocks reflect patched content.
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
                // Patched body is decompressed JSON — upstream gets uncompressed body
                (Bytes::from(patched), true)
            }
            None => (body_bytes.clone(), false),
        }
    } else {
        (body_bytes.clone(), false)
    };

    // Capture from decompressed (and possibly patched) body so blocks parse correctly.
    let capture_body = if body_was_patched {
        &forwarded_body // already decompressed JSON
    } else {
        body_for_processing // decompressed (or raw if not zstd)
    };
    let parsed = state
        .capture
        .capture_request(request_id, path, capture_body);

    // Emit request captured event
    if let (Some(ref dispatcher), Some(ref parsed)) = (&state.dispatcher, &parsed) {
        dispatcher.request_captured(request_id, method, path, &parsed.provider.to_string());
    }

    // Build upstream request
    let mut upstream_req = state.client.request(parts.method, upstream_url);

    // Forward headers, stripping hop-by-hop and proxy-unsafe headers:
    // - Host: reqwest sets the correct Host from the upstream URL
    // - Content-Length: reqwest recalculates from the actual body
    // - Connection: hop-by-hop header, not meant to traverse proxies
    // - Accept-Encoding: prevents upstream from compressing responses,
    //   which avoids decompression conflicts in the proxy's byte-passthrough
    //   pipeline (fixes ChatGPT backend stream disconnects)
    // - Content-Encoding: stripped when body was patched (decompressed),
    //   since the forwarded body is now uncompressed JSON
    for (key, value) in parts.headers.iter() {
        if key == header::HOST
            || key == header::CONTENT_LENGTH
            || key == header::CONNECTION
            || key == header::ACCEPT_ENCODING
        {
            continue;
        }
        // When patches modified a compressed body, the forwarded body is
        // decompressed JSON — don't tell upstream it's still compressed.
        if body_was_patched && key == header::CONTENT_ENCODING {
            debug!("Stripping Content-Encoding header (body was decompressed for patching)");
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

    let upstream_send_start = Instant::now();
    // Send request (with potentially patched body)
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

    // Check if this is a streaming response (SSE)
    let is_streaming = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    if is_streaming {
        debug!("Streaming SSE response");

        // Tee the stream: forward chunks to client AND accumulate for capture
        let request_id_owned = request_id.to_string();
        let upstream_url_owned = upstream_url.to_string();
        let state_clone = state.clone();
        let mut upstream_stream = upstream_response.bytes_stream();
        let stream_start = request_start;
        let upstream_latency_ms = upstream_latency.as_millis() as u64;

        let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

        // Spawn a task to read from upstream and fan out
        tokio::spawn(async move {
            let mut total_bytes: u64 = 0;
            let mut chunk_count: u64 = 0;
            let mut stream_error: Option<String> = None;

            while let Some(chunk_result) = upstream_stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        chunk_count += 1;
                        // Accumulate for capture
                        let bytes_received = state_clone
                            .capture
                            .append_sse_chunk(&request_id_owned, &chunk);
                        total_bytes = bytes_received;

                        // Emit streaming progress (throttled — every 4KB)
                        if let Some(ref dispatcher) = state_clone.dispatcher {
                            if bytes_received % 4096 < chunk.len() as u64 {
                                dispatcher.response_streaming(&request_id_owned, bytes_received);
                            }
                        }

                        // Forward to client
                        if tx.send(Ok(chunk)).await.is_err() {
                            debug!(
                                request_id = %request_id_owned,
                                "Client disconnected during SSE stream"
                            );
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
                    request_id = %request_id_owned,
                    upstream = %upstream_url_owned,
                    bytes_received = total_bytes,
                    chunks = chunk_count,
                    elapsed_ms = stream_start.elapsed().as_millis() as u64,
                    error = %err,
                    "SSE stream disconnected with error"
                );
            } else if total_bytes == 0 {
                error!(
                    request_id = %request_id_owned,
                    upstream = %upstream_url_owned,
                    elapsed_ms = stream_start.elapsed().as_millis() as u64,
                    "SSE stream completed with zero bytes (possible upstream rejection)"
                );
            }

            // Stream complete — finalize capture
            if let Some(exchange) = state_clone.capture.finalize_streaming(&request_id_owned) {
                if let Some(ref dispatcher) = state_clone.dispatcher {
                    let total_tokens = exchange
                        .usage
                        .as_ref()
                        .map(|u| u.input_tokens + u.output_tokens);

                    dispatcher.response_complete(&request_id_owned, status.as_u16(), total_tokens);

                    // Emit block data for frontend consumption
                    dispatcher.blocks_captured(&exchange);
                }

                // Feed blocks to engine for processing + persistence.
                // The engine's own emit_context_updated() is the authoritative signal.
                if let Some(ref engine) = state_clone.engine {
                    engine.ingest(
                        &exchange.provider.to_string(),
                        &exchange.model,
                        "proxy",
                        None,
                        exchange.request_blocks.clone(),
                        exchange.response_blocks.clone(),
                    );
                }

                debug!(
                    request_id = %request_id_owned,
                    upstream_latency_ms,
                    total_latency_ms = stream_start.elapsed().as_millis() as u64,
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

        Ok(response)
    } else {
        let response_bytes = upstream_response.bytes().await?;

        // Log response body preview only when debug logging is enabled.
        // Slice raw bytes BEFORE lossy conversion to avoid panicking on
        // multi-byte char boundaries.
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

        // Capture response
        if parsed.is_some() {
            if let Some(exchange) =
                state
                    .capture
                    .capture_response(request_id, status.as_u16(), &response_bytes)
            {
                if let Some(ref dispatcher) = state.dispatcher {
                    let total_tokens = exchange
                        .usage
                        .as_ref()
                        .map(|u| u.input_tokens + u.output_tokens);

                    dispatcher.response_complete(request_id, status.as_u16(), total_tokens);

                    // Emit block data for frontend consumption
                    dispatcher.blocks_captured(&exchange);
                }

                // Feed blocks to engine for processing + persistence.
                // The engine's own emit_context_updated() is the authoritative signal.
                if let Some(ref engine) = state.engine {
                    engine.ingest(
                        &exchange.provider.to_string(),
                        &exchange.model,
                        "proxy",
                        None,
                        exchange.request_blocks.clone(),
                        exchange.response_blocks.clone(),
                    );
                }
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
        *response.headers_mut() = convert_headers(&headers);

        Ok(response)
    }
}

/// Convert reqwest response headers to axum headers, stripping hop-by-hop
/// and proxy-unsafe headers that must NOT be forwarded.
///
/// Critical: reqwest may auto-decompress the body (changing its size) while
/// the original Content-Length header reflects the compressed size. Forwarding
/// it would cause clients to see a Content-Length mismatch and disconnect.
/// Similarly, Transfer-Encoding is between reqwest↔upstream; axum handles
/// the outgoing Transfer-Encoding to the client separately.
fn convert_headers(headers: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut axum_headers = HeaderMap::new();
    for (key, value) in headers.iter() {
        // Skip hop-by-hop and proxy-unsafe response headers
        if key == header::CONTENT_LENGTH
            || key == header::TRANSFER_ENCODING
            || key == header::CONNECTION
        {
            continue;
        }
        if let Ok(name) = axum::http::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
            if let Ok(val) = axum::http::header::HeaderValue::from_bytes(value.as_bytes()) {
                axum_headers.insert(name, val);
            }
        }
    }
    axum_headers
}

/// Log headers at debug level.
fn log_headers(label: &str, headers: &HeaderMap) {
    for (key, value) in headers.iter() {
        if key == header::AUTHORIZATION || key == "x-api-key" {
            debug!("{} header: {} = [REDACTED]", label, key);
        } else {
            debug!("{} header: {} = {:?}", label, key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_upstream_anthropic_header() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "test".parse().unwrap());

        let result = determine_upstream(&config, &headers, "/v1/messages");
        assert_eq!(result.url, "https://api.anthropic.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_openai_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/v1/chat/completions");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_openai_responses_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/v1/responses");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_bearer_with_openai_path() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer sk-test123".parse().unwrap());

        let result = determine_upstream(&config, &headers, "/v1/chat/completions");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_bearer_with_anthropic_path() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer some-token".parse().unwrap());

        let result = determine_upstream(&config, &headers, "/v1/messages");
        assert_eq!(result.url, "https://api.anthropic.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_chatgpt_subscription_token_routes_to_codex_backend() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer session-token-123".parse().unwrap(),
        );

        // Non-sk- token on /responses → ChatGPT Codex backend
        let result = determine_upstream(&config, &headers, "/v1/responses");
        assert_eq!(result.url, "https://chatgpt.com/backend-api/codex");
        assert!(result.is_chatgpt);

        let result = determine_upstream(&config, &headers, "/responses");
        assert_eq!(result.url, "https://chatgpt.com/backend-api/codex");
        assert!(result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_api_key_stays_on_openai() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer sk-proj-abc123".parse().unwrap(),
        );

        // sk- prefixed key → standard OpenAI API
        let result = determine_upstream(&config, &headers, "/v1/responses");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_upstream_config_default() {
        let config = UpstreamConfig::default();
        assert_eq!(config.anthropic_url, "https://api.anthropic.com");
        assert_eq!(config.openai_url, "https://api.openai.com");
        assert_eq!(
            config.chatgpt_codex_url,
            "https://chatgpt.com/backend-api/codex"
        );
    }

    // --- Bare path detection (no /v1/ prefix) ---

    #[test]
    fn test_determine_upstream_bare_responses_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/responses");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_responses_subpath() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/responses/resp_123/cancel");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_bare_chat_completions_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/chat/completions");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    #[test]
    fn test_determine_upstream_bearer_with_bare_responses_path() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer sk-test123".parse().unwrap());

        let result = determine_upstream(&config, &headers, "/responses");
        assert_eq!(result.url, "https://api.openai.com");
        assert!(!result.is_chatgpt);
    }

    // --- Path normalization ---

    #[test]
    fn test_normalize_bare_responses_path() {
        assert_eq!(normalize_api_path("/responses", false), "/v1/responses");
    }

    #[test]
    fn test_normalize_bare_responses_subpath() {
        assert_eq!(
            normalize_api_path("/responses/resp_123", false),
            "/v1/responses/resp_123"
        );
    }

    #[test]
    fn test_normalize_bare_chat_completions_path() {
        assert_eq!(
            normalize_api_path("/chat/completions", false),
            "/v1/chat/completions"
        );
    }

    #[test]
    fn test_normalize_bare_messages_path() {
        assert_eq!(normalize_api_path("/messages", false), "/v1/messages");
    }

    #[test]
    fn test_normalize_already_prefixed_path() {
        assert_eq!(normalize_api_path("/v1/responses", false), "/v1/responses");
        assert_eq!(normalize_api_path("/v1/messages", false), "/v1/messages");
    }

    #[test]
    fn test_normalize_unknown_path_unchanged() {
        assert_eq!(normalize_api_path("/health", false), "/health");
        assert_eq!(normalize_api_path("/v1/models", false), "/v1/models");
    }

    #[test]
    fn test_normalize_chatgpt_upstream_skips_v1_prefix() {
        // ChatGPT backend expects bare paths — no /v1/ added
        assert_eq!(normalize_api_path("/responses", true), "/responses");
        assert_eq!(
            normalize_api_path("/responses/resp_123", true),
            "/responses/resp_123"
        );
    }

    // --- Upstream URL building ---

    #[test]
    fn test_build_upstream_url_preserves_query_params() {
        let url = build_upstream_url(
            "https://api.openai.com",
            "/v1/responses",
            Some("stream=true&foo=bar"),
            false,
        );
        assert_eq!(
            url,
            "https://api.openai.com/v1/responses?stream=true&foo=bar"
        );
    }

    #[test]
    fn test_build_upstream_url_normalizes_bare_path_and_query() {
        let url =
            build_upstream_url("https://api.openai.com", "/responses", Some("stream=true"), false);
        assert_eq!(url, "https://api.openai.com/v1/responses?stream=true");
    }

    #[test]
    fn test_build_upstream_url_chatgpt_keeps_bare_path() {
        let url = build_upstream_url(
            "https://chatgpt.com/backend-api/codex",
            "/responses",
            None,
            true,
        );
        assert_eq!(url, "https://chatgpt.com/backend-api/codex/responses");
    }

    // --- zstd decompression round-trip ---

    #[test]
    fn test_zstd_round_trip_decompression() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "input": [
                { "role": "user", "content": "Hello from zstd" }
            ]
        });
        let json_bytes = serde_json::to_vec(&json).unwrap();

        // Compress with zstd
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(&json_bytes), 3).unwrap();
        assert_ne!(compressed, json_bytes, "Compressed should differ from original");

        // Decompress
        let decompressed =
            zstd::stream::decode_all(std::io::Cursor::new(&compressed)).unwrap();
        assert_eq!(decompressed, json_bytes, "Round-trip should match original");

        // Verify JSON parses correctly
        let parsed: serde_json::Value = serde_json::from_slice(&decompressed).unwrap();
        assert_eq!(parsed["input"][0]["content"], "Hello from zstd");
    }

    #[test]
    fn test_zstd_content_encoding_detection() {
        let mut headers = HeaderMap::new();
        assert!(
            !headers
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .map(|ce| ce.contains(CONTENT_ENCODING_ZSTD))
                .unwrap_or(false),
            "No content-encoding header should not detect zstd"
        );

        headers.insert(header::CONTENT_ENCODING, "zstd".parse().unwrap());
        assert!(
            headers
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .map(|ce| ce.contains(CONTENT_ENCODING_ZSTD))
                .unwrap_or(false),
            "Should detect zstd content-encoding"
        );

        // Also handle compound encodings like "zstd, identity"
        headers.insert(header::CONTENT_ENCODING, "zstd, identity".parse().unwrap());
        assert!(
            headers
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .map(|ce| ce.contains(CONTENT_ENCODING_ZSTD))
                .unwrap_or(false),
            "Should detect zstd in compound content-encoding"
        );
    }

    #[test]
    fn test_hot_patch_on_decompressed_zstd_body() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "input": [
                { "role": "user", "content": "Hello from codex via zstd" }
            ]
        });
        let json_bytes = serde_json::to_vec(&json).unwrap();

        // Compress
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(&json_bytes), 3).unwrap();

        // Hot patch should fail on compressed bytes
        let patches = vec![hot_patch::HotPatch {
            role: "user".to_string(),
            original_content: "Hello from codex via zstd".to_string(),
            new_content: "Patched via zstd".to_string(),
            source: hot_patch::PatchSource::Manual,
        }];
        assert!(
            hot_patch::apply_patches(&compressed, &patches).is_none(),
            "Patches should not apply to compressed bytes"
        );

        // Decompress then patch
        let decompressed =
            zstd::stream::decode_all(std::io::Cursor::new(&compressed)).unwrap();
        let patched = hot_patch::apply_patches(&decompressed, &patches);
        assert!(patched.is_some(), "Patches should apply to decompressed bytes");

        let patched_json: serde_json::Value =
            serde_json::from_slice(&patched.unwrap()).unwrap();
        assert_eq!(patched_json["input"][0]["content"], "Patched via zstd");
    }
}
