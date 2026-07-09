//! Metadata loader: fetch models.dev catalog JSON, parse, cache to disk.
//!
//! Source: `https://models.dev/catalog.json` — community-maintained at
//! `github.com/anomalyco/models.dev`. The `catalog` endpoint combines:
//! - `providers.<provider>.models.<model>` — provider serving details (cost,
//!   limits, capability flags) — the part we persist into the catalog.
//! - `models.<provider/model>` — provider-agnostic facts (benchmarks, weights,
//!   licenses) — unused for now but kept around so we don't need a second
//!   fetch when we want to surface those later.
//!
//! `ModelEntry` mirrors the upstream shape exactly (cost/limit/modalities are
//! nested structs); the per-1M-token → per-token rescale happens at the
//! `ModelEntry::cost_for` accessor, not here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::core::error::AppError;

use super::catalog::{ModelCatalogSnapshot, ModelEntry};

const MODELS_DEV_URL: &str = "https://models.dev/catalog.json";

const FETCH_TIMEOUT_MS: u64 = 10000;

/// Bounded retry budget for `fetch_metadata`. Worst case sums to ~6 hours:
/// the first 11 attempts ramp from 1s → 1024s (~17 min total), then each
/// remaining attempt waits 30 min, capped at 22 attempts so we can't collide
/// with the 24h refresh tick. Long enough to ride out hours-long outages;
/// short enough that the next scheduled refresh still gets a fresh shot.
const RETRY_MAX_ATTEMPTS: usize = 22;
const RETRY_MIN_DELAY_MS: u64 = 1000;
const RETRY_MAX_DELAY_MS: u64 = 1_800_000;

pub async fn fetch_metadata() -> Result<String, AppError> {
    let backoff = ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(RETRY_MIN_DELAY_MS))
        .with_max_delay(Duration::from_millis(RETRY_MAX_DELAY_MS))
        .with_factor(2.0)
        .with_max_times(RETRY_MAX_ATTEMPTS);

    (|| async { fetch_remote(MODELS_DEV_URL, FETCH_TIMEOUT_MS).await })
        .retry(backoff)
        .sleep(tokio::time::sleep)
        .notify(|err, dur| {
            tracing::warn!(
                error = %err,
                retry_in = ?dur,
                "Model metadata fetch failed, retrying"
            );
        })
        .await
}

pub async fn fetch_remote(url: &str, timeout_ms: u64) -> Result<String, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| AppError::Internal(format!("metadata http client: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("metadata fetch: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "metadata fetch HTTP {}",
            resp.status()
        )));
    }
    resp.text()
        .await
        .map_err(|e| AppError::Internal(format!("metadata fetch body: {e}")))
}

/// Top-level shape of models.dev `catalog.json`. We only need the providers
/// half today; the `models` top-level (provider-agnostic metadata: benchmarks,
/// weights, licenses) is intentionally ignored.
#[derive(Debug, Deserialize)]
struct CatalogJson {
    providers: HashMap<String, ProviderBlock>,
}

#[derive(Debug, Deserialize)]
struct ProviderBlock {
    #[serde(default)]
    models: HashMap<String, ModelEntry>,
}

pub fn parse(json: &str) -> Result<ModelCatalogSnapshot, AppError> {
    let catalog: CatalogJson = serde_json::from_str(json)
        .map_err(|e| AppError::Internal(format!("metadata parse: {e}")))?;

    let mut entries = HashMap::new();
    for (provider_id, block) in catalog.providers {
        for (model_id, entry) in block.models {
            // Skip entries with neither input nor output pricing — open-weights
            // stubs, sample placeholders, embedding-only modes. We'd rather
            // miss the cost than silently zero it.
            if !entry.has_pricing() {
                continue;
            }
            entries.insert(format!("{provider_id}/{model_id}"), entry);
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    let digest = hasher.finalize();
    let version = format!("{:x}", digest)[..12].to_string();

    Ok(ModelCatalogSnapshot {
        version,
        fetched_at: Utc::now(),
        entries,
    })
}

/// Persisted under `Config.storage.cache_dir` so a restart survives the
/// network being down.
const CACHE_FILE_NAME: &str = "models_dev_catalog.json";

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CACHE_FILE_NAME)
}

/// Persist a successful fetch to disk. Errors are non-fatal — the catalog is
/// already swapped in memory; failing to persist just means the next restart
/// won't have a head start.
pub fn save_cache(cache_dir: &Path, raw_json: &str) -> Result<(), AppError> {
    std::fs::create_dir_all(cache_dir).map_err(|e| {
        AppError::Internal(format!("metadata cache mkdir {cache_dir:?}: {e}"))
    })?;
    let path = cache_path(cache_dir);
    std::fs::write(&path, raw_json)
        .map_err(|e| AppError::Internal(format!("metadata cache write {path:?}: {e}")))
}

/// Age of the on-disk cache file, or `None` if the file is missing or its
/// mtime can't be read. Used by the scheduler to skip the startup refresh
/// when the cache is younger than the periodic refresh interval — avoids
/// re-fetching ~2.5 MB on every restart.
pub fn cache_age(cache_dir: &Path) -> Option<Duration> {
    let path = cache_path(cache_dir);
    let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
    mtime.elapsed().ok()
}

/// Boot-time loader: prefer the on-disk cache (from a previous successful
/// refresh) over the hardcoded defaults. Cache miss or parse failure falls
/// back to `ModelCatalogSnapshot::defaults()`, which carries context windows
/// and capability flags for the models frona ships with. The first scheduler
/// refresh overwrites this with fresh models.dev data.
pub fn load_cache_or_defaults(cache_dir: &Path) -> ModelCatalogSnapshot {
    let path = cache_path(cache_dir);
    match std::fs::read_to_string(&path) {
        Ok(json) => match parse(&json) {
            Ok(snapshot) => {
                tracing::info!(
                    path = %path.display(),
                    version = %snapshot.version,
                    entries = snapshot.entries.len(),
                    "Loaded model metadata from cache"
                );
                snapshot
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Cached metadata failed to parse; using defaults"
                );
                ModelCatalogSnapshot::defaults()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(path = %path.display(), "No cached metadata yet; using defaults");
            ModelCatalogSnapshot::defaults()
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Cached metadata read failed; using defaults"
            );
            ModelCatalogSnapshot::defaults()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture mirroring the models.dev catalog shape. Asserts that we
    /// deserialize directly into nested `Cost`/`Limit`/`Modalities` blocks,
    /// the composite-key shape (`{provider}/{model}`), and the cost-less
    /// filter.
    const SAMPLE: &str = r#"{
        "models": {},
        "providers": {
            "anthropic": {
                "id": "anthropic",
                "models": {
                    "claude-opus-4-7": {
                        "id": "claude-opus-4-7",
                        "attachment": true,
                        "reasoning": true,
                        "tool_call": true,
                        "open_weights": false,
                        "structured_output": true,
                        "modalities": {"input": ["text", "image"], "output": ["text"]},
                        "limit": {"context": 1000000, "output": 128000},
                        "cost": {"input": 5, "output": 25, "cache_read": 0.5, "cache_write": 6.25}
                    },
                    "no-cost-stub": {
                        "id": "no-cost-stub",
                        "attachment": false,
                        "reasoning": false,
                        "tool_call": false,
                        "open_weights": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 8192, "output": 4096}
                    }
                }
            },
            "openai": {
                "id": "openai",
                "models": {
                    "gpt-4o": {
                        "id": "gpt-4o",
                        "attachment": true,
                        "reasoning": false,
                        "tool_call": true,
                        "open_weights": false,
                        "structured_output": true,
                        "modalities": {"input": ["text", "image"], "output": ["text"]},
                        "limit": {"context": 128000, "output": 16384},
                        "cost": {"input": 2.5, "output": 10}
                    }
                }
            }
        }
    }"#;

    fn usage(input: u64, output: u64, cached: u64) -> rig_core::completion::request::Usage {
        rig_core::completion::request::Usage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
            cached_input_tokens: cached,
            cache_creation_input_tokens: 0,
            reasoning_tokens: 0,
        }
    }

    #[test]
    fn parse_deserializes_nested_blocks() {
        let snapshot = parse(SAMPLE).expect("parse");
        let opus = snapshot
            .entries
            .get("anthropic/claude-opus-4-7")
            .expect("opus entry");
        let cost = opus.cost.as_ref().expect("cost");
        // Stored as published — USD per 1M tokens, no rescale at this layer.
        assert_eq!(cost.input, 5.0);
        assert_eq!(cost.output, 25.0);
        assert_eq!(cost.cache_read, Some(0.5));
        assert_eq!(cost.cache_write, Some(6.25));
        assert_eq!(opus.limit.context, 1_000_000);
        assert_eq!(opus.limit.output, 128_000);
    }

    #[test]
    fn parse_keys_entries_by_composite_provider_model() {
        let snapshot = parse(SAMPLE).expect("parse");
        assert!(snapshot.entries.contains_key("anthropic/claude-opus-4-7"));
        assert!(snapshot.entries.contains_key("openai/gpt-4o"));
        // Cost-less stub filtered out by has_pricing().
        assert!(!snapshot.entries.contains_key("anthropic/no-cost-stub"));
    }

    #[test]
    fn cost_for_rescales_per_million_to_per_token() {
        let snapshot = parse(SAMPLE).expect("parse");
        let opus = snapshot.entries.get("anthropic/claude-opus-4-7").unwrap();
        // Convention: input_tokens is fresh. 1M * $5/M + 0.5M * $25/M = $17.5.
        let total = opus.cost_for(&usage(1_000_000, 500_000, 0)).expect("cost");
        assert!((total - 17.5).abs() < 1e-9);
    }

    fn model_ref(provider: &str, model_id: &str) -> crate::inference::provider::ModelRef {
        crate::inference::provider::ModelRef {
            provider: provider.into(),
            model_id: model_id.into(),
            additional_params: None,
        }
    }

    #[test]
    fn catalog_normalizes_openai_convention_inputs_at_compute_boundary() {
        use crate::inference::metadata::ModelCatalogStore;

        let store = ModelCatalogStore::new(parse(SAMPLE).expect("parse"));
        // openai convention: input_tokens (1M) is total prompt tokens with
        // 400k of those being cache reads. Catalog must subtract cached from
        // input before reaching cost_for. Cost = 600k * $2.5/M (fresh) + 0
        // (no cache_read rate in fixture) + 0 output = $1.5.
        let (cost, _) = store.compute(&model_ref("openai", "gpt-4o"), &usage(1_000_000, 0, 400_000));
        let cost = cost.expect("cost");
        assert!((cost - 1.5).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn catalog_leaves_anthropic_inputs_alone() {
        use crate::inference::metadata::ModelCatalogStore;

        let store = ModelCatalogStore::new(parse(SAMPLE).expect("parse"));
        // Anthropic convention: input_tokens (600k) is ALREADY fresh; cached
        // (400k) is additive. 600k * $5/M + 400k * $0.5/M = $3.2.
        let (cost, _) = store.compute(
            &model_ref("anthropic", "claude-opus-4-7"),
            &usage(600_000, 0, 400_000),
        );
        let cost = cost.expect("cost");
        assert!((cost - 3.2).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn max_input_tokens_subtracts_output_reservation() {
        let snapshot = parse(SAMPLE).expect("parse");
        let opus = snapshot.entries.get("anthropic/claude-opus-4-7").unwrap();
        // 1_000_000 context - 128_000 output = 872_000 input budget.
        assert_eq!(opus.max_input_tokens(), Some(872_000));
        assert_eq!(opus.max_output_tokens(), Some(128_000));
    }

    #[test]
    fn capability_helpers_derive_correctly() {
        let snapshot = parse(SAMPLE).expect("parse");
        let opus = snapshot.entries.get("anthropic/claude-opus-4-7").unwrap();
        assert!(opus.supports_function_calling());
        assert!(opus.supports_vision());
        assert!(opus.supports_prompt_caching());
        assert!(opus.supports_reasoning());
        assert!(opus.supports_response_schema());

        let gpt = snapshot.entries.get("openai/gpt-4o").unwrap();
        // No cache_read / cache_write in cost block → no caching support.
        assert!(!gpt.supports_prompt_caching());
        // No reasoning flag → no reasoning.
        assert!(!gpt.supports_reasoning());
    }
}
