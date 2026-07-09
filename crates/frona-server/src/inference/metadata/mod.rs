//! Model metadata: pricing + capability flags + context window limits, sourced
//! from [models.dev](https://models.dev/catalog.json) (community-maintained at
//! `github.com/anomalyco/models.dev`) and refreshed by the scheduler. Used by
//! `UsageService` for cost computation; other callers can read capability
//! flags off `ModelEntry` directly.
//!
//! Costs are stored as USD per token. models.dev publishes them as USD per
//! 1M tokens — the parser rescales by `1e-6` on load.

pub mod catalog;
pub mod loader;

pub use catalog::{ModelCatalogSnapshot, ModelCatalogStore, ModelEntry};
