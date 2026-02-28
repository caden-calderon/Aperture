//! Compile-time build fingerprint constants.
//!
//! Populated by `build.rs` via `cargo:rustc-env`. All three binaries
//! (aperture, aperture-proxy, aperture-mcp) share these values.

pub const GIT_HASH: &str = env!("APERTURE_GIT_HASH");
pub const BUILD_TIME: &str = env!("APERTURE_BUILD_TIME");
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Human-readable one-line fingerprint.
///
/// Example: `"aperture 0.1.0 (b432113 built 2026-02-27T14:30:00Z)"`
pub fn fingerprint() -> String {
    format!("aperture {PKG_VERSION} ({GIT_HASH} built {BUILD_TIME})")
}

/// Structured version info for the `/_aperture/version` endpoint.
pub fn version_json() -> serde_json::Value {
    serde_json::json!({
        "version": PKG_VERSION,
        "git_hash": GIT_HASH,
        "build_time": BUILD_TIME,
        "fingerprint": fingerprint()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_hash_format() {
        // 7 hex chars, optionally followed by "-dirty"
        let re = regex::Regex::new(r"^[0-9a-f]{7}(-dirty)?$").unwrap();
        assert!(
            re.is_match(GIT_HASH) || GIT_HASH == "unknown",
            "unexpected git hash format: {GIT_HASH}"
        );
    }

    #[test]
    fn build_time_format() {
        // ISO 8601 UTC: 2026-02-27T14:30:00Z
        let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").unwrap();
        assert!(
            re.is_match(BUILD_TIME) || BUILD_TIME == "unknown",
            "unexpected build time format: {BUILD_TIME}"
        );
    }

    #[test]
    fn fingerprint_contains_all_parts() {
        let fp = fingerprint();
        assert!(fp.contains(PKG_VERSION), "fingerprint missing version");
        assert!(fp.contains(GIT_HASH), "fingerprint missing git hash");
        assert!(fp.contains(BUILD_TIME), "fingerprint missing build time");
        assert!(fp.starts_with("aperture "), "fingerprint missing prefix");
    }

    #[test]
    fn version_json_structure() {
        let v = version_json();
        assert_eq!(v["version"], PKG_VERSION);
        assert_eq!(v["git_hash"], GIT_HASH);
        assert_eq!(v["build_time"], BUILD_TIME);
        assert!(v["fingerprint"].as_str().unwrap().contains(GIT_HASH));
    }
}
