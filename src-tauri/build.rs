use std::process::Command;

fn main() {
    // ── Build fingerprint: git hash + timestamp ──────────────────────
    //
    // Emitted as cargo:rustc-env so all binaries (aperture, aperture-proxy,
    // aperture-mcp) share the same compile-time constants via env!().

    // Git short hash with -dirty suffix if working tree has changes.
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

    let git_hash = if dirty {
        format!("{git_hash}-dirty")
    } else {
        git_hash
    };

    // UTC build timestamp (ISO 8601).
    let build_time = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=APERTURE_GIT_HASH={git_hash}");
    println!("cargo:rustc-env=APERTURE_BUILD_TIME={build_time}");

    // Rerun when git state changes. build.rs runs from src-tauri/,
    // but .git/ is at the project root (one level up).
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");

    tauri_build::build()
}
