use super::headers::{
    connection_header_tokens, convert_headers, has_zstd_content_encoding,
    should_strip_request_header,
};
use super::routing::{build_upstream_url, determine_upstream, normalize_api_path};
use crate::proxy::{hot_patch, UpstreamConfig};
use axum::http::{header, HeaderMap};
use std::collections::HashSet;

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

    // Non-sk- token on /responses -> ChatGPT Codex backend
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

    // sk- prefixed key -> standard OpenAI API
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
    // ChatGPT backend expects bare paths - no /v1/ added
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
