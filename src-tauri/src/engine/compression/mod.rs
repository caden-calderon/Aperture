//! Phase 4 compression foundations.
//!
//! This module defines the sidekick compression configuration contract and
//! default provider/model routing policy. Runtime integration (queue workers
//! and network calls) can build on these types incrementally.

pub mod provider;
pub mod queue;

use serde::{Deserialize, Serialize};

/// Compression backend selection strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompressionBackendKind {
    /// Pick backend from the active upstream provider.
    #[default]
    Auto,
    /// Disable sidekick compression (fail-open, no sidekick requests).
    Disabled,
    Anthropic,
    OpenAi,
    OpenRouter,
}

/// Sidekick compression runtime settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompressionSettings {
    pub backend: CompressionBackendKind,
    /// Optional model override. When absent, defaults are provider-aware.
    pub model: Option<String>,
    /// Backend request timeout in milliseconds.
    pub timeout_ms: u64,
    /// Target maximum summary tokens for sidekick generations.
    pub max_tokens: u32,
    /// Optional OpenRouter base URL override.
    pub openrouter_base_url: Option<String>,
    /// Optional environment variable name containing OpenRouter API key.
    pub openrouter_api_key_env: Option<String>,
}

impl Default for CompressionSettings {
    fn default() -> Self {
        Self {
            backend: CompressionBackendKind::Auto,
            model: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_tokens: DEFAULT_MAX_TOKENS,
            openrouter_base_url: None,
            openrouter_api_key_env: Some(DEFAULT_OPENROUTER_KEY_ENV.to_string()),
        }
    }
}

impl CompressionSettings {
    /// Normalize settings from user input.
    pub fn normalized(self) -> Self {
        Self {
            backend: self.backend,
            model: clean_optional(self.model),
            timeout_ms: self.timeout_ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS),
            max_tokens: self.max_tokens.clamp(MIN_MAX_TOKENS, MAX_MAX_TOKENS),
            openrouter_base_url: clean_optional(self.openrouter_base_url),
            openrouter_api_key_env: clean_optional(self.openrouter_api_key_env),
        }
    }

    /// Resolve backend for a given active provider name.
    pub fn resolved_backend(&self, active_provider: &str) -> CompressionBackendKind {
        match self.backend {
            CompressionBackendKind::Auto => default_backend_for_provider(active_provider),
            explicit => explicit,
        }
    }

    /// Resolve model for a given active provider.
    ///
    /// Returns `None` when sidekick compression is disabled.
    pub fn resolved_model(&self, active_provider: &str) -> Option<String> {
        let backend = self.resolved_backend(active_provider);
        if backend == CompressionBackendKind::Disabled {
            return None;
        }
        if let Some(model) = clean_optional(self.model.clone()) {
            return Some(model);
        }
        Some(default_model_for_backend(backend).to_string())
    }
}

/// Default backend mapping by active provider.
pub fn default_backend_for_provider(active_provider: &str) -> CompressionBackendKind {
    let provider = active_provider.trim().to_ascii_lowercase();
    match provider.as_str() {
        "anthropic" => CompressionBackendKind::Anthropic,
        "openai" | "chatgpt" | "codex" => CompressionBackendKind::OpenAi,
        "gemini" => CompressionBackendKind::OpenRouter,
        _ => CompressionBackendKind::Anthropic,
    }
}

/// Default sidekick model for a resolved backend.
pub fn default_model_for_backend(backend: CompressionBackendKind) -> &'static str {
    match backend {
        CompressionBackendKind::Auto => "claude-3-5-haiku-latest",
        CompressionBackendKind::Disabled => "",
        CompressionBackendKind::Anthropic => "claude-3-5-haiku-latest",
        CompressionBackendKind::OpenAi => "gpt-5-mini",
        CompressionBackendKind::OpenRouter => "google/gemini-2.0-flash-001",
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

const DEFAULT_TIMEOUT_MS: u64 = 12_000;
const MIN_TIMEOUT_MS: u64 = 500;
const MAX_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_TOKENS: u32 = 512;
const MIN_MAX_TOKENS: u32 = 64;
const MAX_MAX_TOKENS: u32 = 8_192;
const DEFAULT_OPENROUTER_KEY_ENV: &str = "OPENROUTER_API_KEY";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_backend_routing() {
        assert_eq!(
            default_backend_for_provider("anthropic"),
            CompressionBackendKind::Anthropic
        );
        assert_eq!(
            default_backend_for_provider("openai"),
            CompressionBackendKind::OpenAi
        );
        assert_eq!(
            default_backend_for_provider("chatgpt"),
            CompressionBackendKind::OpenAi
        );
        assert_eq!(
            default_backend_for_provider("gemini"),
            CompressionBackendKind::OpenRouter
        );
        assert_eq!(
            default_backend_for_provider("unknown"),
            CompressionBackendKind::Anthropic
        );
    }

    #[test]
    fn test_settings_normalized_clamps_and_trims() {
        let settings = CompressionSettings {
            backend: CompressionBackendKind::OpenRouter,
            model: Some("  custom-model  ".to_string()),
            timeout_ms: 5,
            max_tokens: 99_999,
            openrouter_base_url: Some("  https://openrouter.ai/api/v1  ".to_string()),
            openrouter_api_key_env: Some("   ".to_string()),
        }
        .normalized();

        assert_eq!(settings.timeout_ms, MIN_TIMEOUT_MS);
        assert_eq!(settings.max_tokens, MAX_MAX_TOKENS);
        assert_eq!(settings.model.as_deref(), Some("custom-model"));
        assert_eq!(
            settings.openrouter_base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(settings.openrouter_api_key_env.is_none());
    }

    #[test]
    fn test_resolved_model_uses_override_when_present() {
        let settings = CompressionSettings {
            backend: CompressionBackendKind::Auto,
            model: Some("my-model".to_string()),
            ..CompressionSettings::default()
        };

        assert_eq!(
            settings.resolved_model("anthropic").as_deref(),
            Some("my-model")
        );
    }

    #[test]
    fn test_resolved_model_uses_provider_defaults_when_missing() {
        let settings = CompressionSettings::default();

        assert_eq!(
            settings.resolved_model("openai").as_deref(),
            Some("gpt-5-mini")
        );
        assert_eq!(
            settings.resolved_model("anthropic").as_deref(),
            Some("claude-3-5-haiku-latest")
        );
    }

    #[test]
    fn test_resolved_model_returns_none_when_disabled() {
        let settings = CompressionSettings {
            backend: CompressionBackendKind::Disabled,
            ..CompressionSettings::default()
        };
        assert!(settings.resolved_model("openai").is_none());
    }
}
