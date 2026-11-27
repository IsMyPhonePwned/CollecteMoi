//! Release version shown in admin UI /status and `--version`.
//!
//! Resolution order:
//! 1. Runtime `COLLECTEMOI_VERSION` (compose/env override)
//! 2. Build-time `COLLECTEMOI_VERSION` (Docker/CI → `COLLECTEMOI_BUILD_VERSION`)
//! 3. Cargo package version (`Cargo.toml`)

/// Baked at compile time (may be empty).
const BUILD_VERSION: &str = env!("COLLECTEMOI_BUILD_VERSION");

/// Static version for clap `--version` (no runtime env).
pub const VERSION: &str = {
    if BUILD_VERSION.is_empty() {
        env!("CARGO_PKG_VERSION")
    } else {
        BUILD_VERSION
    }
};

/// Version string for API / admin UI (honours runtime override).
pub fn displayed() -> String {
    std::env::var("COLLECTEMOI_VERSION")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| VERSION.to_string())
}
