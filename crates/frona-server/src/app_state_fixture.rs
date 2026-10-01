//! Test-support helper for building the `CatalogSources` argument
//! `AppState::new` needs, without any of its own tests reaching out to
//! `models.dev`/`modelparams.dev`. Not gated behind `#[cfg(test)]`: both this
//! crate's own `#[cfg(test)]` modules and the separate integration-test
//! binaries under `tests/` need to call it, and only the former sees
//! `cfg(test)` items from this crate.

use crate::core::config::Config;

/// Load whatever is already on disk at `config.storage.cache_dir` (nothing,
/// in a fresh test tempdir) — never fetches over the network.
pub fn catalogs(config: &Config) -> frona_model_catalog::sources::CatalogSources {
    frona_model_catalog::sources::CatalogSources::load(std::path::Path::new(
        &config.storage.cache_dir,
    ))
}
