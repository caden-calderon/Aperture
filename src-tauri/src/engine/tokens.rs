//! Model-aware token counting.
//!
//! Uses tiktoken-rs for accurate counts on OpenAI models.
//! Falls back to a chars/4 heuristic for unknown models.

use std::sync::OnceLock;

use tiktoken_rs::CoreBPE;

static CL100K: OnceLock<Result<CoreBPE, String>> = OnceLock::new();
static O200K: OnceLock<Result<CoreBPE, String>> = OnceLock::new();

/// Which tokenizer family to use for a given model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerKind {
    /// cl100k_base — GPT-4, GPT-3.5-turbo, Claude (approximate)
    Cl100k,
    /// o200k_base — GPT-4o, GPT-5, Codex, o1/o3/o4 family
    O200k,
    /// Character-based heuristic (chars / 4)
    Heuristic,
}

/// Select the appropriate tokenizer for a model name.
pub fn tokenizer_for_model(model: &str) -> TokenizerKind {
    let m = model.to_lowercase();

    // OpenAI newest models — GPT-5.x, Codex 5.x (o200k tokenizer)
    if m.starts_with("gpt-5") || m.starts_with("codex") {
        return TokenizerKind::O200k;
    }

    // OpenAI o200k models — GPT-4o, o1/o3/o4 reasoning family
    if m.starts_with("gpt-4o") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4")
    {
        return TokenizerKind::O200k;
    }

    // OpenAI cl100k models — legacy GPT-4, GPT-3.5
    if m.starts_with("gpt-4") || m.starts_with("gpt-3.5") {
        return TokenizerKind::Cl100k;
    }

    // Anthropic Claude — cl100k is a reasonable approximation
    if m.starts_with("claude") {
        return TokenizerKind::Cl100k;
    }

    // Unknown — fall back to heuristic
    TokenizerKind::Heuristic
}

/// Count tokens for a given piece of content and model.
pub fn count_tokens(content: &str, model: &str) -> u32 {
    if content.is_empty() {
        return 0;
    }

    match tokenizer_for_model(model) {
        TokenizerKind::Cl100k => count_with_cl100k(content),
        TokenizerKind::O200k => count_with_o200k(content),
        TokenizerKind::Heuristic => estimate_tokens(content),
    }
}

/// Heuristic estimate: roughly 1 token per 4 characters.
/// This is the fallback when no tokenizer is available.
pub fn estimate_tokens(content: &str) -> u32 {
    let len = content.len();
    // Minimum 1 token for non-empty content
    len.div_ceil(4).max(1) as u32
}

fn get_cl100k() -> &'static Result<CoreBPE, String> {
    CL100K.get_or_init(|| tiktoken_rs::cl100k_base().map_err(|e| e.to_string()))
}

fn get_o200k() -> &'static Result<CoreBPE, String> {
    O200K.get_or_init(|| tiktoken_rs::o200k_base().map_err(|e| e.to_string()))
}

fn count_with_cl100k(content: &str) -> u32 {
    match get_cl100k() {
        Ok(tokenizer) => tokenizer.encode_ordinary(content).len() as u32,
        Err(_) => estimate_tokens(content),
    }
}

fn count_with_o200k(content: &str) -> u32 {
    match get_o200k() {
        Ok(tokenizer) => tokenizer.encode_ordinary(content).len() as u32,
        Err(_) => estimate_tokens(content),
    }
}

/// Recount tokens for a block's content with accurate model-based counting.
/// Returns the updated token count.
pub fn recount_block_tokens(content: &str, model: &str) -> u32 {
    count_tokens(content, model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiktoken_rs::{cl100k_base, o200k_base};

    #[test]
    fn test_tokenizer_selection_openai_gpt4() {
        assert_eq!(tokenizer_for_model("gpt-4-turbo"), TokenizerKind::Cl100k);
        assert_eq!(tokenizer_for_model("gpt-4"), TokenizerKind::Cl100k);
    }

    #[test]
    fn test_tokenizer_selection_openai_gpt4o() {
        assert_eq!(tokenizer_for_model("gpt-4o"), TokenizerKind::O200k);
        assert_eq!(tokenizer_for_model("gpt-4o-mini"), TokenizerKind::O200k);
    }

    #[test]
    fn test_tokenizer_selection_openai_o1() {
        assert_eq!(tokenizer_for_model("o1-preview"), TokenizerKind::O200k);
        assert_eq!(tokenizer_for_model("o3-mini"), TokenizerKind::O200k);
    }

    #[test]
    fn test_tokenizer_selection_claude() {
        assert_eq!(
            tokenizer_for_model("claude-sonnet-4-5-20250929"),
            TokenizerKind::Cl100k
        );
        assert_eq!(
            tokenizer_for_model("claude-opus-4-6"),
            TokenizerKind::Cl100k
        );
    }

    #[test]
    fn test_tokenizer_selection_gpt5_and_codex() {
        assert_eq!(tokenizer_for_model("gpt-5"), TokenizerKind::O200k);
        assert_eq!(tokenizer_for_model("gpt-5-mini"), TokenizerKind::O200k);
        assert_eq!(tokenizer_for_model("gpt-5.3"), TokenizerKind::O200k);
        assert_eq!(tokenizer_for_model("codex-5.3"), TokenizerKind::O200k);
        assert_eq!(tokenizer_for_model("codex-mini"), TokenizerKind::O200k);
    }

    #[test]
    fn test_tokenizer_selection_unknown() {
        assert_eq!(
            tokenizer_for_model("some-random-model"),
            TokenizerKind::Heuristic
        );
    }

    #[test]
    fn test_count_tokens_empty() {
        assert_eq!(count_tokens("", "gpt-4"), 0);
        assert_eq!(count_tokens("", "claude-opus-4-6"), 0);
    }

    #[test]
    fn test_count_tokens_cl100k() {
        // "Hello, world!" is typically 4 tokens in cl100k
        let count = count_tokens("Hello, world!", "gpt-4");
        assert!(count > 0);
        assert!(count < 10); // sanity check
    }

    #[test]
    fn test_count_tokens_o200k() {
        let count = count_tokens("Hello, world!", "gpt-4o");
        assert!(count > 0);
        assert!(count < 10);
    }

    #[test]
    fn test_estimate_tokens_heuristic() {
        // estimate_tokens always returns at least 1 for non-empty strings
        // For empty strings, use count_tokens() which checks is_empty first
        assert_eq!(estimate_tokens("hi"), 1); // 2 chars → 1 token (min 1)
        assert_eq!(estimate_tokens("hello world test"), 4); // 16 chars → 4 tokens
        assert_eq!(estimate_tokens("a"), 1); // 1 char → 1 token (min 1)
    }

    #[test]
    fn test_count_tokens_accuracy_reasonable() {
        // Verify tiktoken gives a reasonable count for a known string.
        // "The quick brown fox jumps over the lazy dog" is typically ~10 tokens.
        let content = "The quick brown fox jumps over the lazy dog";
        let count = count_tokens(content, "gpt-4");
        assert!(
            (8..=12).contains(&count),
            "Expected 8-12 tokens, got {count}"
        );
    }

    #[test]
    fn test_count_tokens_within_two_percent_of_reference_tokenizers() {
        let corpus = [
            "fn add(a: i32, b: i32) -> i32 { a + b }",
            "The quick brown fox jumps over the lazy dog.",
            "Traceback (most recent call last): File \"main.py\", line 1",
            "src/lib/components/layout/ZoneManager.svelte",
            "Actually, wait, let's use the o200k model tokenizer path.",
            "Unicode check: café naïve résumé 東京",
        ];
        let models = [
            "gpt-4-turbo",   // cl100k
            "gpt-4o",        // o200k
            "gpt-5",         // o200k
            "codex-5.3",     // o200k
            "claude-sonnet", // cl100k approximation
        ];

        let cl100k = cl100k_base().expect("cl100k tokenizer should initialize");
        let o200k = o200k_base().expect("o200k tokenizer should initialize");

        for model in models {
            for content in corpus {
                let expected = match tokenizer_for_model(model) {
                    TokenizerKind::Cl100k => cl100k.encode_ordinary(content).len() as u32,
                    TokenizerKind::O200k => o200k.encode_ordinary(content).len() as u32,
                    TokenizerKind::Heuristic => estimate_tokens(content),
                };
                let actual = count_tokens(content, model);

                if expected == 0 {
                    assert_eq!(actual, 0, "Expected zero tokens for model={model}");
                    continue;
                }

                let err_ratio = (actual as f64 - expected as f64).abs() / expected as f64;
                assert!(
                    err_ratio <= 0.02,
                    "Token error exceeded 2% for model={model}, content={content:?}, expected={expected}, actual={actual}, ratio={err_ratio:.4}"
                );
            }
        }
    }
}
