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

    let upstream_base = determine_upstream(&state.config, req.headers(), &path);
    let upstream_url = build_upstream_url(upstream_base, &path, uri.query());

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

/// Determine which upstream to use based on request characteristics.
///
/// Supports both `/v1/` prefixed and bare paths (e.g. Codex sends `/responses`).
fn determine_upstream<'a>(config: &'a UpstreamConfig, headers: &HeaderMap, path: &str) -> &'a str {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

    // Check for Anthropic-specific header
    if headers.contains_key("x-api-key") || headers.contains_key("anthropic-version") {
        return &config.anthropic_url;
    }

    // Check for OpenAI-style authorization (Bearer token, not checking prefix per spec)
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth.to_str() {
            if auth_str.starts_with("Bearer ") {
                // Path-based disambiguation for bearer tokens
                if is_chat_completions_path(path) || is_responses_path(path) {
                    return &config.openai_url;
                }
                if is_messages_path(path) {
                    return &config.anthropic_url;
                }
                // Default bearer to OpenAI (most common case for generic bearer)
                return &config.openai_url;
            }
        }
    }

    // Path-based detection (fallback for requests without auth headers)
    if is_messages_path(path) {
        return &config.anthropic_url;
    }
    if is_chat_completions_path(path) || is_responses_path(path) {
        return &config.openai_url;
    }

    // Default to Anthropic (primary use case)
    &config.anthropic_url
}

/// Normalize bare API paths by adding the `/v1/` prefix if missing.
///
/// Codex and some tools send requests to `/responses` or `/chat/completions`
/// without the `/v1/` prefix. Upstream APIs (OpenAI, Anthropic) require it.
fn normalize_api_path(path: &str) -> String {
    use super::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

    // Only normalize if it's a known API path WITHOUT /v1/ prefix
    if (is_messages_path(path) || is_chat_completions_path(path) || is_responses_path(path))
        && !path.starts_with("/v1/")
    {
        format!("/v1{path}")
    } else {
        path.to_string()
    }
}

/// Build upstream URL from base + normalized API path, preserving query params.
fn build_upstream_url(upstream_base: &str, path: &str, query: Option<&str>) -> String {
    let normalized_path = normalize_api_path(path);
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

    // Log request body preview only when debug logging is enabled.
    if tracing::enabled!(Level::DEBUG) && !body_bytes.is_empty() {
        let body_preview = String::from_utf8_lossy(&body_bytes);
        let preview = if body_preview.len() > 500 {
            format!(
                "{}... ({} bytes total)",
                &body_preview[..500],
                body_bytes.len()
            )
        } else {
            body_preview.to_string()
        };
        debug!("Request body: {}", preview);
    }

    // Apply hot patches BEFORE capture so blocks reflect patched content
    let pending_patches = state.hot_patches.drain();
    let forwarded_body = if !pending_patches.is_empty() {
        match hot_patch::apply_patches(&body_bytes, &pending_patches) {
            Some(patched) => {
                debug!(
                    "Hot patches applied, body modified ({} -> {} bytes)",
                    body_bytes.len(),
                    patched.len()
                );
                Bytes::from(patched)
            }
            None => body_bytes.clone(),
        }
    } else {
        body_bytes.clone()
    };

    // Capture from the (potentially patched) body so frontend sees post-patch content
    let parsed = state
        .capture
        .capture_request(request_id, path, &forwarded_body);

    // Emit request captured event
    if let (Some(ref dispatcher), Some(ref parsed)) = (&state.dispatcher, &parsed) {
        dispatcher.request_captured(request_id, method, path, &parsed.provider.to_string());
    }

    // Build upstream request
    let mut upstream_req = state.client.request(parts.method, upstream_url);

    // Forward headers (except host and content-length — reqwest recalculates the latter)
    for (key, value) in parts.headers.iter() {
        if key != header::HOST && key != header::CONTENT_LENGTH {
            upstream_req = upstream_req.header(key, value);
        }
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
        let state_clone = state.clone();
        let mut upstream_stream = upstream_response.bytes_stream();
        let stream_start = request_start;
        let upstream_latency_ms = upstream_latency.as_millis() as u64;

        let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

        // Spawn a task to read from upstream and fan out
        tokio::spawn(async move {
            while let Some(chunk_result) = upstream_stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        // Accumulate for capture
                        let bytes_received = state_clone
                            .capture
                            .append_sse_chunk(&request_id_owned, &chunk);

                        // Emit streaming progress (throttled — every 4KB)
                        if let Some(ref dispatcher) = state_clone.dispatcher {
                            if bytes_received % 4096 < chunk.len() as u64 {
                                dispatcher.response_streaming(&request_id_owned, bytes_received);
                            }
                        }

                        // Forward to client
                        if tx.send(Ok(chunk)).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(std::io::Error::other(e.to_string()))).await;
                        break;
                    }
                }
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

                    let total_blocks =
                        (exchange.request_blocks.len() + exchange.response_blocks.len()) as u32;
                    let total_token_sum: u32 = exchange
                        .request_blocks
                        .iter()
                        .chain(exchange.response_blocks.iter())
                        .map(|b| b.tokens)
                        .sum();
                    dispatcher.context_updated(total_blocks, total_token_sum);
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
        if tracing::enabled!(Level::DEBUG) {
            let body_preview = String::from_utf8_lossy(&response_bytes);
            let preview = if body_preview.len() > 500 {
                format!(
                    "{}... ({} bytes total)",
                    &body_preview[..500],
                    response_bytes.len()
                )
            } else {
                body_preview.to_string()
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

                    let total_blocks =
                        (exchange.request_blocks.len() + exchange.response_blocks.len()) as u32;
                    let total_token_sum: u32 = exchange
                        .request_blocks
                        .iter()
                        .chain(exchange.response_blocks.iter())
                        .map(|b| b.tokens)
                        .sum();
                    dispatcher.context_updated(total_blocks, total_token_sum);
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

/// Convert reqwest headers to axum headers.
fn convert_headers(headers: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut axum_headers = HeaderMap::new();
    for (key, value) in headers.iter() {
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
        assert_eq!(result, "https://api.anthropic.com");
    }

    #[test]
    fn test_determine_upstream_openai_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/v1/chat/completions");
        assert_eq!(result, "https://api.openai.com");
    }

    #[test]
    fn test_determine_upstream_openai_responses_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/v1/responses");
        assert_eq!(result, "https://api.openai.com");
    }

    #[test]
    fn test_determine_upstream_bearer_with_openai_path() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer sk-test123".parse().unwrap());

        let result = determine_upstream(&config, &headers, "/v1/chat/completions");
        assert_eq!(result, "https://api.openai.com");
    }

    #[test]
    fn test_determine_upstream_bearer_with_anthropic_path() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer some-token".parse().unwrap());

        let result = determine_upstream(&config, &headers, "/v1/messages");
        assert_eq!(result, "https://api.anthropic.com");
    }

    #[test]
    fn test_determine_upstream_bearer_without_sk_prefix_openai_path() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer session-token-123".parse().unwrap(),
        );

        let result = determine_upstream(&config, &headers, "/v1/responses");
        assert_eq!(result, "https://api.openai.com");
    }

    #[test]
    fn test_upstream_config_default() {
        let config = UpstreamConfig::default();
        assert_eq!(config.anthropic_url, "https://api.anthropic.com");
        assert_eq!(config.openai_url, "https://api.openai.com");
    }

    // --- Bare path detection (no /v1/ prefix) ---

    #[test]
    fn test_determine_upstream_bare_responses_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/responses");
        assert_eq!(result, "https://api.openai.com");
    }

    #[test]
    fn test_determine_upstream_responses_subpath() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/responses/resp_123/cancel");
        assert_eq!(result, "https://api.openai.com");
    }

    #[test]
    fn test_determine_upstream_bare_chat_completions_path() {
        let config = UpstreamConfig::default();
        let headers = HeaderMap::new();

        let result = determine_upstream(&config, &headers, "/chat/completions");
        assert_eq!(result, "https://api.openai.com");
    }

    #[test]
    fn test_determine_upstream_bearer_with_bare_responses_path() {
        let config = UpstreamConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer sk-test123".parse().unwrap());

        let result = determine_upstream(&config, &headers, "/responses");
        assert_eq!(result, "https://api.openai.com");
    }

    // --- Path normalization ---

    #[test]
    fn test_normalize_bare_responses_path() {
        assert_eq!(normalize_api_path("/responses"), "/v1/responses");
    }

    #[test]
    fn test_normalize_bare_responses_subpath() {
        assert_eq!(
            normalize_api_path("/responses/resp_123"),
            "/v1/responses/resp_123"
        );
    }

    #[test]
    fn test_normalize_bare_chat_completions_path() {
        assert_eq!(
            normalize_api_path("/chat/completions"),
            "/v1/chat/completions"
        );
    }

    #[test]
    fn test_normalize_bare_messages_path() {
        assert_eq!(normalize_api_path("/messages"), "/v1/messages");
    }

    #[test]
    fn test_normalize_already_prefixed_path() {
        assert_eq!(normalize_api_path("/v1/responses"), "/v1/responses");
        assert_eq!(normalize_api_path("/v1/messages"), "/v1/messages");
    }

    #[test]
    fn test_normalize_unknown_path_unchanged() {
        assert_eq!(normalize_api_path("/health"), "/health");
        assert_eq!(normalize_api_path("/v1/models"), "/v1/models");
    }

    // --- Upstream URL building ---

    #[test]
    fn test_build_upstream_url_preserves_query_params() {
        let url = build_upstream_url(
            "https://api.openai.com",
            "/v1/responses",
            Some("stream=true&foo=bar"),
        );
        assert_eq!(
            url,
            "https://api.openai.com/v1/responses?stream=true&foo=bar"
        );
    }

    #[test]
    fn test_build_upstream_url_normalizes_bare_path_and_query() {
        let url = build_upstream_url("https://api.openai.com", "/responses", Some("stream=true"));
        assert_eq!(url, "https://api.openai.com/v1/responses?stream=true");
    }
}
