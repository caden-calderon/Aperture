//! Lightweight guardrails for runaway request patterns.
//!
//! Keeps rolling counters for high-frequency proxy and context-tool traffic,
//! and emits advisory alerts without blocking request forwarding (fail-open).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PROXY_WINDOW: Duration = Duration::from_secs(120);
const CONTEXT_WINDOW: Duration = Duration::from_secs(60);
const WARNING_COOLDOWN: Duration = Duration::from_secs(30);

const PROXY_REQUEST_THRESHOLD: usize = 8;
const PROXY_LARGE_BODY_THRESHOLD: usize = 4;
const PROXY_LARGE_BODY_BYTES: usize = 40_000;

const CONTEXT_TOOL_THRESHOLD: usize = 12;
const CONTEXT_CIRCUIT_BREAKER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunawayAlert {
    pub channel: &'static str,
    pub count: usize,
    pub window_secs: u64,
    pub threshold: usize,
    pub hard_limit: bool,
}

#[derive(Debug, Default)]
pub struct RunawayGuard {
    state: Mutex<RunawayState>,
}

#[derive(Debug, Default)]
struct RunawayState {
    proxy_requests: VecDeque<ProxySample>,
    context_tool_calls: VecDeque<Instant>,
    context_circuit_open_until: Option<Instant>,
    last_proxy_warning: Option<Instant>,
    last_context_warning: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct ProxySample {
    at: Instant,
    is_large: bool,
}

impl RunawayGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_proxy_request(&self, body_bytes: usize) -> Option<RunawayAlert> {
        let now = Instant::now();
        let mut state = self.state.lock().expect("runaway guard poisoned");

        state.proxy_requests.push_back(ProxySample {
            at: now,
            is_large: body_bytes >= PROXY_LARGE_BODY_BYTES,
        });
        prune_proxy_window(&mut state.proxy_requests, now);

        let request_count = state.proxy_requests.len();
        let large_count = state
            .proxy_requests
            .iter()
            .filter(|sample| sample.is_large)
            .count();
        let should_alert =
            request_count >= PROXY_REQUEST_THRESHOLD && large_count >= PROXY_LARGE_BODY_THRESHOLD;

        if !should_alert {
            return None;
        }

        if in_cooldown(state.last_proxy_warning, now) {
            return None;
        }
        state.last_proxy_warning = Some(now);

        Some(RunawayAlert {
            channel: "proxy_request_rate",
            count: request_count,
            window_secs: PROXY_WINDOW.as_secs(),
            threshold: PROXY_REQUEST_THRESHOLD,
            hard_limit: request_count >= PROXY_REQUEST_THRESHOLD * 2,
        })
    }

    pub fn record_context_tool_call(&self) -> Option<RunawayAlert> {
        let now = Instant::now();
        let mut state = self.state.lock().expect("runaway guard poisoned");

        state.context_tool_calls.push_back(now);
        prune_instant_window(&mut state.context_tool_calls, now, CONTEXT_WINDOW);

        let count = state.context_tool_calls.len();
        let hard_limit = count >= CONTEXT_TOOL_THRESHOLD * 2;
        if hard_limit {
            state.context_circuit_open_until = Some(now + CONTEXT_CIRCUIT_BREAKER);
        }

        if count < CONTEXT_TOOL_THRESHOLD {
            return None;
        }

        if in_cooldown(state.last_context_warning, now) && !hard_limit {
            return None;
        }
        state.last_context_warning = Some(now);

        Some(RunawayAlert {
            channel: "context_tool_rate",
            count,
            window_secs: CONTEXT_WINDOW.as_secs(),
            threshold: CONTEXT_TOOL_THRESHOLD,
            hard_limit,
        })
    }

    /// Remaining duration of an active context-tools circuit breaker.
    pub fn context_circuit_remaining(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut state = self.state.lock().expect("runaway guard poisoned");
        let until = state.context_circuit_open_until?;
        if now >= until {
            state.context_circuit_open_until = None;
            return None;
        }
        Some(until.saturating_duration_since(now))
    }
}

fn prune_proxy_window(samples: &mut VecDeque<ProxySample>, now: Instant) {
    while let Some(sample) = samples.front().copied() {
        if now.saturating_duration_since(sample.at) > PROXY_WINDOW {
            samples.pop_front();
        } else {
            break;
        }
    }
}

fn prune_instant_window(samples: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while let Some(sample) = samples.front().copied() {
        if now.saturating_duration_since(sample) > window {
            samples.pop_front();
        } else {
            break;
        }
    }
}

fn in_cooldown(last_warning: Option<Instant>, now: Instant) -> bool {
    last_warning
        .map(|last| now.saturating_duration_since(last) < WARNING_COOLDOWN)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_alert_requires_burst_with_large_payloads() {
        let guard = RunawayGuard::new();
        for _ in 0..(PROXY_REQUEST_THRESHOLD - 1) {
            assert!(guard
                .record_proxy_request(PROXY_LARGE_BODY_BYTES + 1)
                .is_none());
        }

        let alert = guard
            .record_proxy_request(PROXY_LARGE_BODY_BYTES + 1)
            .expect("expected proxy burst alert");
        assert_eq!(alert.channel, "proxy_request_rate");
        assert!(!alert.hard_limit);
    }

    #[test]
    fn context_alert_triggers_at_threshold() {
        let guard = RunawayGuard::new();
        for _ in 0..(CONTEXT_TOOL_THRESHOLD - 1) {
            assert!(guard.record_context_tool_call().is_none());
        }

        let alert = guard
            .record_context_tool_call()
            .expect("expected context tool alert");
        assert_eq!(alert.channel, "context_tool_rate");
        assert!(!alert.hard_limit);
    }

    #[test]
    fn context_hard_limit_is_reported() {
        let guard = RunawayGuard::new();
        // first alert
        for _ in 0..CONTEXT_TOOL_THRESHOLD {
            let _ = guard.record_context_tool_call();
        }

        // Force cooldown bypass in test by directly mutating state.
        let mut state = guard.state.lock().expect("lock");
        state.last_context_warning = None;
        drop(state);

        for _ in 0..CONTEXT_TOOL_THRESHOLD {
            let _ = guard.record_context_tool_call();
        }
        let mut state = guard.state.lock().expect("lock");
        state.last_context_warning = None;
        drop(state);

        let alert = guard
            .record_context_tool_call()
            .expect("expected hard-limit alert");
        assert!(alert.hard_limit);
        assert!(
            guard.context_circuit_remaining().is_some(),
            "hard limit should open context circuit breaker"
        );
    }
}
