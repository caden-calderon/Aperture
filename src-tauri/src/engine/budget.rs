//! Budget threshold alerts.
//!
//! Monitors token usage against the session budget and triggers
//! alerts at configurable thresholds.

use serde::{Deserialize, Serialize};

/// Budget alert severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    /// Under 80% — normal operation.
    Normal,
    /// 80-89% — approaching limit.
    Warning,
    /// 90-94% — near limit.
    Critical,
    /// 95%+ — emergency, compression/removal recommended.
    Emergency,
}

/// Full budget status report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub used_tokens: u32,
    pub limit_tokens: u32,
    pub utilization: f64,
    pub alert_level: AlertLevel,
    pub remaining_tokens: u32,
}

/// Budget threshold configuration.
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    pub warning_threshold: f64,
    pub critical_threshold: f64,
    pub emergency_threshold: f64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            warning_threshold: 0.80,
            critical_threshold: 0.90,
            emergency_threshold: 0.95,
        }
    }
}

/// Check budget utilization and return the appropriate alert level.
pub fn check_budget(used: u32, limit: u32, config: &BudgetConfig) -> AlertLevel {
    if limit == 0 {
        return AlertLevel::Normal;
    }

    let utilization = f64::from(used) / f64::from(limit);

    if utilization >= config.emergency_threshold {
        AlertLevel::Emergency
    } else if utilization >= config.critical_threshold {
        AlertLevel::Critical
    } else if utilization >= config.warning_threshold {
        AlertLevel::Warning
    } else {
        AlertLevel::Normal
    }
}

/// Compute a full budget status report.
pub fn budget_status(used: u32, limit: u32, config: &BudgetConfig) -> BudgetStatus {
    let utilization = if limit > 0 {
        f64::from(used) / f64::from(limit)
    } else {
        0.0
    };

    BudgetStatus {
        used_tokens: used,
        limit_tokens: limit,
        utilization,
        alert_level: check_budget(used, limit, config),
        remaining_tokens: limit.saturating_sub(used),
    }
}

/// Default token budget per provider/model.
pub fn default_token_limit(model: &str) -> u32 {
    let m = model.to_lowercase();

    // Anthropic Claude family
    if m.contains("claude") {
        return 200_000;
    }

    // OpenAI Codex 5.x (newest — 256k context)
    if m.starts_with("codex") {
        return 256_000;
    }

    // OpenAI GPT-5.x (200k context)
    if m.starts_with("gpt-5") {
        return 200_000;
    }

    // OpenAI reasoning models (o1, o3, o4 — 200k context)
    if m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") {
        return 200_000;
    }

    // OpenAI GPT-4o / GPT-4 (128k context)
    if m.starts_with("gpt-4") {
        return 128_000;
    }

    128_000 // safe default
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_levels() {
        let config = BudgetConfig::default();

        assert_eq!(check_budget(50_000, 200_000, &config), AlertLevel::Normal); // 25%
        assert_eq!(check_budget(160_000, 200_000, &config), AlertLevel::Warning); // 80%
        assert_eq!(
            check_budget(182_000, 200_000, &config),
            AlertLevel::Critical
        ); // 91%
        assert_eq!(
            check_budget(195_000, 200_000, &config),
            AlertLevel::Emergency
        ); // 97.5%
    }

    #[test]
    fn test_budget_status() {
        let config = BudgetConfig::default();
        let status = budget_status(160_000, 200_000, &config);

        assert_eq!(status.used_tokens, 160_000);
        assert_eq!(status.limit_tokens, 200_000);
        assert!((status.utilization - 0.8).abs() < f64::EPSILON);
        assert_eq!(status.alert_level, AlertLevel::Warning);
        assert_eq!(status.remaining_tokens, 40_000);
    }

    #[test]
    fn test_zero_limit_is_normal() {
        let config = BudgetConfig::default();
        assert_eq!(check_budget(100, 0, &config), AlertLevel::Normal);
    }

    #[test]
    fn test_default_token_limits() {
        // Anthropic
        assert_eq!(default_token_limit("claude-opus-4-6"), 200_000);
        assert_eq!(default_token_limit("claude-sonnet-4-5-20250929"), 200_000);
        // OpenAI newest
        assert_eq!(default_token_limit("codex-5.3"), 256_000);
        assert_eq!(default_token_limit("codex-mini"), 256_000);
        assert_eq!(default_token_limit("gpt-5"), 200_000);
        assert_eq!(default_token_limit("gpt-5.3"), 200_000);
        assert_eq!(default_token_limit("gpt-5-mini"), 200_000);
        // OpenAI reasoning
        assert_eq!(default_token_limit("o1-preview"), 200_000);
        assert_eq!(default_token_limit("o3-mini"), 200_000);
        assert_eq!(default_token_limit("o4-mini"), 200_000);
        // OpenAI legacy
        assert_eq!(default_token_limit("gpt-4o-mini"), 128_000);
        assert_eq!(default_token_limit("gpt-4-turbo"), 128_000);
    }
}
