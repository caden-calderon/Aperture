//! Shared utility functions used across proxy, engine, and terminal modules.

use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Load environment from `.env` file (if present).
///
/// Searches project root and parent directories. No-op if no `.env` found.
pub fn load_env() {
    let paths = [".env", "../.env", "../../.env"];
    for path in paths {
        if dotenvy::from_filename(path).is_ok() {
            return;
        }
    }
}

/// Initialize tracing/logging with sensible defaults.
///
/// Debug builds show `aperture_lib=debug`, release builds show `info` only.
/// Respects `RUST_LOG` env var when set.
pub fn init_logging() {
    let default_filter = if cfg!(debug_assertions) {
        "info,aperture_lib=debug"
    } else {
        "info"
    };

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| default_filter.into()))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Get configured proxy port from environment or default (5400).
pub fn get_proxy_port() -> u16 {
    std::env::var("APERTURE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(crate::proxy::DEFAULT_PORT)
}

/// Install a global panic hook that writes crash diagnostics to a file.
///
/// Captures panics from ANY thread and persists the backtrace to
/// `/tmp/aperture-crash.log` so crash evidence survives process death.
pub fn install_crash_diagnostics() {
    if std::env::var("RUST_BACKTRACE").is_err() {
        // SAFETY: called before any threads are spawned.
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    std::panic::set_hook(Box::new(|panic_info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");

        let crash_report = format!(
            "=== APERTURE PANIC ===\n\
             Time: {:?}\n\
             Thread: {} ({:?})\n\
             Info: {}\n\
             Location: {:?}\n\
             Backtrace:\n{}\n\
             === END PANIC ===\n\n",
            SystemTime::now(),
            thread_name,
            std::thread::current().id(),
            panic_info,
            panic_info.location(),
            backtrace,
        );

        let crash_path = "/tmp/aperture-crash.log";
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(crash_path)
        {
            let _ = std::io::Write::write_all(&mut file, crash_report.as_bytes());
        }

        eprintln!("{crash_report}");
    }));
}

/// Regex matching ANSI escape sequences: CSI sequences and OSC sequences.
static ANSI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]|\x1b\][^\x07]*\x07").unwrap());

/// Strip ANSI escape codes (color, cursor, OSC) from a string.
pub fn strip_ansi_codes(s: &str) -> String {
    ANSI_RE.replace_all(s, "").into_owned()
}

/// Returns the current UTC time as an ISO 8601 string (e.g., "2026-02-10T14:30:00Z").
///
/// Uses the Howard Hinnant `civil_from_days` algorithm for O(1) date calculation
/// without external dependencies like `chrono`.
pub fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix_secs_to_iso(secs)
}

/// Converts unix seconds to ISO 8601 string.
fn unix_secs_to_iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hours = rem / 3600;
    let minutes = (rem % 3600) / 60;
    let seconds = rem % 60;

    // Howard Hinnant's civil_from_days algorithm
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iso_now_format() {
        let ts = iso_now();
        // Should match YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }

    #[test]
    fn test_known_epoch() {
        // 2026-01-01T00:00:00Z = 1767225600 unix seconds
        assert_eq!(unix_secs_to_iso(1_767_225_600), "2026-01-01T00:00:00Z");
    }

    #[test]
    fn test_unix_epoch() {
        assert_eq!(unix_secs_to_iso(0), "1970-01-01T00:00:00Z");
    }

    // ── ANSI Stripping ──────────────────────────────────────

    #[test]
    fn test_strip_ansi_color_codes() {
        let input = "\x1b[31mError:\x1b[0m file not found";
        assert_eq!(strip_ansi_codes(input), "Error: file not found");
    }

    #[test]
    fn test_strip_ansi_noop_on_clean_string() {
        let input = "No escape codes here";
        assert_eq!(strip_ansi_codes(input), input);
    }

    #[test]
    fn test_strip_ansi_mixed_codes() {
        let input = "\x1b[1;32m✓\x1b[0m Pass \x1b[33mwarning\x1b[0m end";
        assert_eq!(strip_ansi_codes(input), "✓ Pass warning end");
    }

    #[test]
    fn test_strip_ansi_empty_string() {
        assert_eq!(strip_ansi_codes(""), "");
    }

    #[test]
    fn test_strip_ansi_codes_only() {
        let input = "\x1b[31m\x1b[0m";
        assert_eq!(strip_ansi_codes(input), "");
    }
}
