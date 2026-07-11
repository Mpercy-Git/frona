pub mod config;
pub mod error;
pub mod event_bus;
pub mod handle;
pub mod log_stream;
pub mod metadata;
pub mod metrics;
pub mod principal;
pub mod repository;
pub mod shutdown;
pub mod state;
pub mod supervisor;
pub mod template;

pub use handle::Handle;
pub use principal::{Principal, PrincipalKind};

/// The application version reported at runtime.
///
/// Prefers the `FRONA_VERSION` environment variable — set on the release image
/// via a Docker build-arg — so cutting a release doesn't require bumping
/// `Cargo.toml`, which would change every crate's fingerprint and force a full
/// release recompile inside the Docker build. Falls back to the compile-time
/// crate version when the variable is unset, empty, or the `dev` sentinel
/// (the Dockerfile's default when no version is passed), so local and
/// development builds still report the `Cargo.toml` version. A leading `v` (as
/// in a `v2026.7.3` image tag) is stripped so the reported value matches the
/// `Cargo.toml` format.
pub fn app_version() -> String {
    std::env::var("FRONA_VERSION")
        .ok()
        .map(|s| s.trim().trim_start_matches('v').to_string())
        .filter(|s| !s.is_empty() && s != "dev")
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

#[cfg(test)]
mod version_tests {
    use super::app_version;

    // These mutate a process-global env var, so keep them in one test to avoid
    // cross-test interference.
    #[test]
    fn app_version_prefers_env_and_falls_back() {
        let compiled = env!("CARGO_PKG_VERSION");

        unsafe { std::env::remove_var("FRONA_VERSION") };
        assert_eq!(app_version(), compiled, "unset -> compile-time version");

        unsafe { std::env::set_var("FRONA_VERSION", "") };
        assert_eq!(app_version(), compiled, "empty -> compile-time version");

        unsafe { std::env::set_var("FRONA_VERSION", "dev") };
        assert_eq!(app_version(), compiled, "dev sentinel -> compile-time version");

        unsafe { std::env::set_var("FRONA_VERSION", "v2026.9.9") };
        assert_eq!(app_version(), "2026.9.9", "leading v stripped");

        unsafe { std::env::set_var("FRONA_VERSION", "2026.9.9") };
        assert_eq!(app_version(), "2026.9.9");

        unsafe { std::env::remove_var("FRONA_VERSION") };
    }
}
