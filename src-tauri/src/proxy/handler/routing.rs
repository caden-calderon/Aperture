use axum::http::{header, HeaderMap};

use crate::proxy::UpstreamConfig;

/// Upstream routing result — carries both the URL and whether this is a
/// ChatGPT Codex backend route (which needs bare paths, no `/v1/` prefix).
pub(super) struct UpstreamRoute<'a> {
    pub(super) url: &'a str,
    pub(super) is_chatgpt: bool,
}

/// Determine which upstream to use based on request characteristics.
///
/// Supports both `/v1/` prefixed and bare paths (e.g. Codex sends `/responses`).
/// Detects ChatGPT subscription tokens (non-`sk-` Bearer) on Responses API paths
/// and routes to the ChatGPT backend instead of the standard OpenAI API.
pub(super) fn determine_upstream<'a>(
    config: &'a UpstreamConfig,
    headers: &HeaderMap,
    path: &str,
) -> UpstreamRoute<'a> {
    use crate::proxy::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

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
pub(super) fn normalize_api_path(path: &str, is_chatgpt: bool) -> String {
    use crate::proxy::parser::{is_chat_completions_path, is_messages_path, is_responses_path};

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
pub(super) fn build_upstream_url(
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
pub(super) fn is_supported_api_path(path: &str) -> bool {
    use crate::proxy::parser::{is_chat_completions_path, is_messages_path, is_responses_path};
    is_messages_path(path) || is_chat_completions_path(path) || is_responses_path(path)
}
