use axum::http::{header, HeaderMap};
use std::collections::HashSet;
use tracing::debug;

/// Header value for zstd content encoding.
const CONTENT_ENCODING_ZSTD: &str = "zstd";

/// Parse `content-encoding` and detect whether zstd is present.
pub(super) fn has_zstd_content_encoding(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .any(|encoding| encoding.eq_ignore_ascii_case(CONTENT_ENCODING_ZSTD))
}

/// Parse all Connection header values and collect nominated hop-by-hop header names.
pub(super) fn connection_header_tokens(headers: &HeaderMap) -> HashSet<header::HeaderName> {
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

pub(super) fn should_strip_request_header(
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

/// Convert reqwest response headers to axum headers, stripping hop-by-hop
/// and proxy-unsafe headers that must not be forwarded.
///
/// Content-Length and hop-by-hop transport headers belong to the immediate
/// upstream connection only. Forwarding them across the proxy boundary can
/// cause framing mismatches and stream instability.
pub(super) fn convert_headers(headers: &reqwest::header::HeaderMap) -> HeaderMap {
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
pub(super) fn log_headers(label: &str, headers: &HeaderMap) {
    for (key, value) in headers.iter() {
        if key == header::AUTHORIZATION || key == "x-api-key" {
            debug!("{} header: {} = [REDACTED]", label, key);
        } else {
            debug!("{} header: {} = {:?}", label, key, value);
        }
    }
}
