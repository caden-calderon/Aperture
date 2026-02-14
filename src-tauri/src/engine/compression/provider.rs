//! Sidekick compression provider contract.
//!
//! Providers are intentionally fail-open: errors are returned to the caller,
//! and callers can choose to keep original content when sidekick compression
//! is unavailable.

use thiserror::Error;

use super::CompressionSettings;

/// Target summary depth requested from the sidekick backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionTarget {
    Summarized,
    Minimal,
}

/// Compression request payload passed to provider adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionRequest {
    pub content: String,
    pub preserve_keys: Vec<String>,
    pub target: CompressionTarget,
}

/// Provider errors are non-fatal (fail-open contract).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CompressionProviderError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("timeout after {0}ms")]
    Timeout(u64),
    #[error("invalid provider response: {0}")]
    InvalidResponse(String),
}

/// Compression provider interface for sidekick adapters.
pub trait CompressionProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn compress(
        &self,
        request: &CompressionRequest,
        settings: &CompressionSettings,
    ) -> Result<String, CompressionProviderError>;
}

/// Local no-network stub backend used for tests and scaffolding.
#[derive(Debug, Default)]
pub struct StubProvider;

impl CompressionProvider for StubProvider {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn compress(
        &self,
        request: &CompressionRequest,
        _settings: &CompressionSettings,
    ) -> Result<String, CompressionProviderError> {
        // Preserve-key list is intentionally left in-band for future prompt templates.
        let target_tag = match request.target {
            CompressionTarget::Summarized => "summary",
            CompressionTarget::Minimal => "minimal",
        };
        Ok(format!(
            "[{target_tag}] {}",
            request.content.trim().chars().take(120).collect::<String>()
        ))
    }
}

/// Failing provider used to validate fail-open behavior in tests.
#[derive(Debug, Default)]
pub struct FailingProvider;

impl CompressionProvider for FailingProvider {
    fn name(&self) -> &'static str {
        "failing"
    }

    fn compress(
        &self,
        _request: &CompressionRequest,
        settings: &CompressionSettings,
    ) -> Result<String, CompressionProviderError> {
        Err(CompressionProviderError::Timeout(settings.timeout_ms))
    }
}

/// Try sidekick compression and return `None` on any provider failure.
pub fn try_compress_fail_open(
    provider: &dyn CompressionProvider,
    request: &CompressionRequest,
    settings: &CompressionSettings,
) -> Option<String> {
    provider
        .compress(request, settings)
        .ok()
        .and_then(|result| {
            let trimmed = result.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compression::{CompressionBackendKind, CompressionSettings};

    fn request() -> CompressionRequest {
        CompressionRequest {
            content: "fn auth() { return validate(token); }".to_string(),
            preserve_keys: vec!["validate".to_string()],
            target: CompressionTarget::Summarized,
        }
    }

    #[test]
    fn test_stub_provider_returns_summary() {
        let provider = StubProvider;
        let settings = CompressionSettings::default();
        let output = provider.compress(&request(), &settings).expect("summary");
        assert!(output.starts_with("[summary]"));
    }

    #[test]
    fn test_try_compress_fail_open_on_provider_error() {
        let provider = FailingProvider;
        let settings = CompressionSettings {
            backend: CompressionBackendKind::OpenAi,
            timeout_ms: 2500,
            ..CompressionSettings::default()
        };

        let output = try_compress_fail_open(&provider, &request(), &settings);
        assert!(output.is_none());
    }

    #[test]
    fn test_try_compress_fail_open_drops_empty_output() {
        struct EmptyProvider;
        impl CompressionProvider for EmptyProvider {
            fn name(&self) -> &'static str {
                "empty"
            }

            fn compress(
                &self,
                _request: &CompressionRequest,
                _settings: &CompressionSettings,
            ) -> Result<String, CompressionProviderError> {
                Ok("   ".to_string())
            }
        }

        let provider = EmptyProvider;
        let settings = CompressionSettings::default();
        let output = try_compress_fail_open(&provider, &request(), &settings);
        assert!(output.is_none());
    }
}
