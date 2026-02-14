//! Request handler for the proxy.

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashSet;
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
fn determine_upstream<'a>(
    config: &'a UpstreamConfig,
    headers: &HeaderMap,
    path: &str,
) -> UpstreamRoute<'a> {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

    let route = |url: &'a str| UpstreamRoute {
        url,
        is_chatgpt: false,
    };
    let chatgpt = |url: &'a str| UpstreamRoute {
        url,
        is_chatgpt: true,
    };

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

/// Fast path check: only known API endpoints need JSON parsing/capture transforms.
fn is_supported_api_path(path: &str) -> bool {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};
    is_messages_path(path) || is_chat_completions_path(path) || is_responses_path(path)
}

/// Parse `content-encoding` and detect whether zstd is present.
fn has_zstd_content_encoding(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .any(|encoding| encoding.eq_ignore_ascii_case(CONTENT_ENCODING_ZSTD))
}

/// Parse all Connection header values and collect nominated hop-by-hop header names.
fn connection_header_tokens(headers: &HeaderMap) -> HashSet<header::HeaderName> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .filter_map(|token| header::HeaderName::from_bytes(token.as_bytes()).ok())
        .collect()
}

fn is_hop_by_hop_header(name: &header::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "proxy-connection"
    )
}

fn should_strip_request_header(
    name: &header::HeaderName,
    connection_tokens: &HashSet<header::HeaderName>,
    body_was_patched: bool,
) -> bool {
    if connection_tokens.contains(name) {
        return true;
    }

    if is_hop_by_hop_header(name) {
        return true;
    }

    if name == header::HOST || name == header::CONTENT_LENGTH || name == header::ACCEPT_ENCODING {
        return true;
    }

    body_was_patched && name == header::CONTENT_ENCODING
}

fn should_strip_response_header(name: &reqwest::header::HeaderName) -> bool {
    if is_hop_by_hop_header(name) {
        return true;
    }

    name == header::CONTENT_LENGTH
}

/// Dispatch events and feed engine after an exchange completes.
fn finalize_exchange(
    state: &ProxyState,
    request_id: &str,
    status: u16,
    exchange: &super::capture::CapturedExchange,
) {
    if let Some(ref dispatcher) = state.dispatcher {
        let total_tokens = exchange
            .usage
            .as_ref()
            .map(|u| u.input_tokens + u.output_tokens);
        dispatcher.response_complete(request_id, status, total_tokens);
        dispatcher.blocks_captured(exchange);
    }

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
            finalize_exchange(&state, &request_id, status.as_u16(), &exchange);

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
                            finalize_exchange(state, request_id, status.as_u16(), &exchange);
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
            finalize_exchange(state, request_id, status.as_u16(), &exchange);
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

    // Apply context mutations (planner rewriting, tool cleanup, manifest injection).
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

    let (forwarded_body, body_was_patched) = if let Some(ref engine) = state.engine {
        if let Some(ref parsed) = parsed_for_rewrite {
            match super::rewriter::rewrite_request(rewrite_input, path, parsed, engine) {
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
    let parsed = state
        .capture
        .capture_request(request_id, path, capture_body);

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
            if is_hop_by_hop_header(key) {
                debug!("Stripping hop-by-hop request header: {}", key);
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

/// Convert reqwest response headers to axum headers, stripping hop-by-hop
/// and proxy-unsafe headers that must NOT be forwarded.
///
/// Content-Length and hop-by-hop transport headers belong to the immediate
/// upstream connection only. Forwarding them across the proxy boundary can
/// cause framing mismatches and stream instability.
fn convert_headers(headers: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut axum_headers = HeaderMap::new();
    for (key, value) in headers.iter() {
        if should_strip_response_header(key) {
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
        let url = build_upstream_url(
            "https://api.openai.com",
            "/responses",
            Some("stream=true"),
            false,
        );
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

    #[test]
    fn test_connection_header_tokens_parses_nominated_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            "keep-alive, te, x-custom-hop".parse().unwrap(),
        );

        let tokens = connection_header_tokens(&headers);
        assert!(tokens.contains(&header::HeaderName::from_static("keep-alive")));
        assert!(tokens.contains(&header::HeaderName::from_static("te")));
        assert!(tokens.contains(&header::HeaderName::from_static("x-custom-hop")));
    }

    #[test]
    fn test_should_strip_request_header_covers_hop_by_hop_and_transport_headers() {
        let connection_tokens = HashSet::from([header::HeaderName::from_static("x-forward-hop")]);

        assert!(should_strip_request_header(
            &header::CONNECTION,
            &connection_tokens,
            false
        ));
        assert!(should_strip_request_header(
            &header::ACCEPT_ENCODING,
            &connection_tokens,
            false
        ));
        assert!(should_strip_request_header(
            &header::HOST,
            &connection_tokens,
            false
        ));
        assert!(should_strip_request_header(
            &header::HeaderName::from_static("x-forward-hop"),
            &connection_tokens,
            false
        ));
        assert!(should_strip_request_header(
            &header::CONTENT_ENCODING,
            &connection_tokens,
            true
        ));
        assert!(!should_strip_request_header(
            &header::CONTENT_TYPE,
            &connection_tokens,
            false
        ));
    }

    #[test]
    fn test_convert_headers_strips_hop_by_hop_response_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(header::CONNECTION, "keep-alive".parse().unwrap());
        headers.insert("keep-alive", "timeout=5".parse().unwrap());
        headers.insert(header::TRANSFER_ENCODING, "chunked".parse().unwrap());
        headers.insert(header::TE, "trailers".parse().unwrap());
        headers.insert(header::UPGRADE, "websocket".parse().unwrap());
        headers.insert(header::TRAILER, "expires".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "123".parse().unwrap());
        headers.insert("x-aperture-upstream", "ok".parse().unwrap());

        let converted = convert_headers(&headers);
        assert!(!converted.contains_key(header::CONNECTION));
        assert!(!converted.contains_key("keep-alive"));
        assert!(!converted.contains_key(header::TRANSFER_ENCODING));
        assert!(!converted.contains_key(header::TE));
        assert!(!converted.contains_key(header::UPGRADE));
        assert!(!converted.contains_key(header::TRAILER));
        assert!(!converted.contains_key(header::CONTENT_LENGTH));
        assert_eq!(converted.get("x-aperture-upstream").unwrap(), "ok");
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
        assert_ne!(
            compressed, json_bytes,
            "Compressed should differ from original"
        );

        // Decompress
        let decompressed = zstd::stream::decode_all(std::io::Cursor::new(&compressed)).unwrap();
        assert_eq!(decompressed, json_bytes, "Round-trip should match original");

        // Verify JSON parses correctly
        let parsed: serde_json::Value = serde_json::from_slice(&decompressed).unwrap();
        assert_eq!(parsed["input"][0]["content"], "Hello from zstd");
    }

    #[test]
    fn test_zstd_content_encoding_detection() {
        let mut headers = HeaderMap::new();
        assert!(
            !has_zstd_content_encoding(&headers),
            "No content-encoding header should not detect zstd"
        );

        headers.insert(header::CONTENT_ENCODING, "zstd".parse().unwrap());
        assert!(
            has_zstd_content_encoding(&headers),
            "Should detect zstd content-encoding"
        );

        // Also handle compound encodings like "zstd, identity"
        headers.insert(header::CONTENT_ENCODING, "zstd, identity".parse().unwrap());
        assert!(
            has_zstd_content_encoding(&headers),
            "Should detect zstd in compound content-encoding"
        );

        // Ensure tokenized matching is precise.
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        assert!(!has_zstd_content_encoding(&headers));
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
        let decompressed = zstd::stream::decode_all(std::io::Cursor::new(&compressed)).unwrap();
        let patched = hot_patch::apply_patches(&decompressed, &patches);
        assert!(
            patched.is_some(),
            "Patches should apply to decompressed bytes"
        );

        let patched_json: serde_json::Value = serde_json::from_slice(&patched.unwrap()).unwrap();
        assert_eq!(patched_json["input"][0]["content"], "Patched via zstd");
    }
}
