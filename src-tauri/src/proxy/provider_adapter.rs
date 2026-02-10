//! Provider parser adapter contract and builtin implementations.
//!
//! This keeps provider-specific parsing concerns at the proxy boundary and
//! provides a single extension point for future providers.

use serde::{Deserialize, Serialize};

use super::parser::{
    is_chat_completions_path, is_messages_path, is_responses_path, parse_anthropic_request,
    parse_anthropic_response, parse_openai_chat_request, parse_openai_chat_response,
    parse_openai_responses_request, parse_openai_responses_response, ParsedRequest, ParsedResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAdapterId {
    Anthropic,
    OpenAI,
    GeminiCli,
    OpenCode,
    KiloCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderParserCapabilities {
    pub supports_usage: bool,
    pub supports_reasoning: bool,
    pub supports_resume_id: bool,
}

pub trait ProviderParserAdapter: Send + Sync {
    fn id(&self) -> ProviderAdapterId;
    fn display_name(&self) -> &'static str;
    fn capabilities(&self) -> ProviderParserCapabilities;
    fn matches_path(&self, path: &str) -> bool;
    fn parse_request(&self, path: &str, body: &[u8]) -> Result<ParsedRequest, String>;
    fn parse_response(&self, path: &str, body: &[u8]) -> Result<ParsedResponse, String>;
}

#[derive(Debug, Default)]
pub struct AnthropicParserAdapter;

impl ProviderParserAdapter for AnthropicParserAdapter {
    fn id(&self) -> ProviderAdapterId {
        ProviderAdapterId::Anthropic
    }

    fn display_name(&self) -> &'static str {
        "Anthropic"
    }

    fn capabilities(&self) -> ProviderParserCapabilities {
        ProviderParserCapabilities {
            supports_usage: true,
            supports_reasoning: true,
            supports_resume_id: false,
        }
    }

    fn matches_path(&self, path: &str) -> bool {
        is_messages_path(path)
    }

    fn parse_request(&self, _path: &str, body: &[u8]) -> Result<ParsedRequest, String> {
        parse_anthropic_request(body)
    }

    fn parse_response(&self, _path: &str, body: &[u8]) -> Result<ParsedResponse, String> {
        parse_anthropic_response(body)
    }
}

#[derive(Debug, Default)]
pub struct OpenAIParserAdapter;

impl ProviderParserAdapter for OpenAIParserAdapter {
    fn id(&self) -> ProviderAdapterId {
        ProviderAdapterId::OpenAI
    }

    fn display_name(&self) -> &'static str {
        "OpenAI"
    }

    fn capabilities(&self) -> ProviderParserCapabilities {
        ProviderParserCapabilities {
            supports_usage: true,
            supports_reasoning: true,
            supports_resume_id: true,
        }
    }

    fn matches_path(&self, path: &str) -> bool {
        is_chat_completions_path(path) || is_responses_path(path)
    }

    fn parse_request(&self, path: &str, body: &[u8]) -> Result<ParsedRequest, String> {
        if is_responses_path(path) {
            parse_openai_responses_request(body)
        } else {
            parse_openai_chat_request(body)
        }
    }

    fn parse_response(&self, path: &str, body: &[u8]) -> Result<ParsedResponse, String> {
        if is_responses_path(path) {
            parse_openai_responses_response(body)
        } else {
            parse_openai_chat_response(body)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PlaceholderParserAdapter {
    id: ProviderAdapterId,
    display_name: &'static str,
    capabilities: ProviderParserCapabilities,
}

impl PlaceholderParserAdapter {
    pub const fn new(
        id: ProviderAdapterId,
        display_name: &'static str,
        capabilities: ProviderParserCapabilities,
    ) -> Self {
        Self {
            id,
            display_name,
            capabilities,
        }
    }
}

impl ProviderParserAdapter for PlaceholderParserAdapter {
    fn id(&self) -> ProviderAdapterId {
        self.id
    }

    fn display_name(&self) -> &'static str {
        self.display_name
    }

    fn capabilities(&self) -> ProviderParserCapabilities {
        self.capabilities
    }

    fn matches_path(&self, _path: &str) -> bool {
        false
    }

    fn parse_request(&self, _path: &str, _body: &[u8]) -> Result<ParsedRequest, String> {
        Err(format!(
            "{} parser adapter is not implemented yet",
            self.display_name
        ))
    }

    fn parse_response(&self, _path: &str, _body: &[u8]) -> Result<ParsedResponse, String> {
        Err(format!(
            "{} parser adapter is not implemented yet",
            self.display_name
        ))
    }
}

pub fn builtin_parser_adapters() -> Vec<Box<dyn ProviderParserAdapter>> {
    vec![
        Box::<AnthropicParserAdapter>::default(),
        Box::<OpenAIParserAdapter>::default(),
        Box::new(PlaceholderParserAdapter::new(
            ProviderAdapterId::GeminiCli,
            "Gemini CLI",
            ProviderParserCapabilities {
                supports_usage: true,
                supports_reasoning: true,
                supports_resume_id: true,
            },
        )),
        Box::new(PlaceholderParserAdapter::new(
            ProviderAdapterId::OpenCode,
            "OpenCode",
            ProviderParserCapabilities {
                supports_usage: false,
                supports_reasoning: true,
                supports_resume_id: true,
            },
        )),
        Box::new(PlaceholderParserAdapter::new(
            ProviderAdapterId::KiloCode,
            "KiloCode",
            ProviderParserCapabilities {
                supports_usage: false,
                supports_reasoning: false,
                supports_resume_id: false,
            },
        )),
    ]
}

pub fn detect_adapter_for_path(path: &str) -> Option<ProviderAdapterId> {
    for adapter in builtin_parser_adapters() {
        if adapter.matches_path(path) {
            return Some(adapter.id());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_adapter_for_anthropic_messages_path() {
        assert_eq!(
            detect_adapter_for_path("/v1/messages"),
            Some(ProviderAdapterId::Anthropic)
        );
    }

    #[test]
    fn test_detect_adapter_for_openai_responses_path() {
        assert_eq!(
            detect_adapter_for_path("/responses"),
            Some(ProviderAdapterId::OpenAI)
        );
    }

    #[test]
    fn test_detect_adapter_for_unknown_path_returns_none() {
        assert_eq!(detect_adapter_for_path("/v1/models"), None);
    }

    #[test]
    fn test_placeholder_adapters_exist_for_phase2_expansion() {
        let ids: Vec<ProviderAdapterId> =
            builtin_parser_adapters().iter().map(|a| a.id()).collect();
        assert!(ids.contains(&ProviderAdapterId::GeminiCli));
        assert!(ids.contains(&ProviderAdapterId::OpenCode));
        assert!(ids.contains(&ProviderAdapterId::KiloCode));
    }

    #[test]
    fn test_openai_capabilities_include_resume_id() {
        let adapter = OpenAIParserAdapter;
        let caps = adapter.capabilities();
        assert!(caps.supports_usage);
        assert!(caps.supports_resume_id);
    }
}
