//! Mirrors the models.dev catalog shape directly so the loader is a one-step
//! deserialize. All `Cost` fields are USD per **1M tokens** as published
//! upstream; `ModelEntry::cost_for` handles the per-token rescale internally.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use rig_core::completion::request::Usage;
use serde::Deserialize;

use crate::inference::provider::ModelRef;

/// Required-vs-optional matches the upstream Zod schema in
/// `github.com/anomalyco/models.dev` `packages/core/src/schema.ts`.
/// Parsing fails-loud if upstream drops a required field.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct ModelEntry {
    pub attachment: bool,
    pub reasoning: bool,
    pub tool_call: bool,
    pub open_weights: bool,
    pub limit: Limit,
    pub modalities: Modalities,

    /// Optional upstream — open-weights models without hosted pricing.
    #[serde(default)]
    pub cost: Option<Cost>,

    #[serde(default)]
    pub structured_output: bool,
    #[serde(default)]
    pub temperature: bool,

    /// `"alpha" | "beta" | "deprecated"`; `None` means generally available.
    #[serde(default)]
    pub status: Option<String>,

    /// YYYY-MM-DD or YYYY-MM when published.
    #[serde(default)]
    pub knowledge: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    /// Anthropic-only; other providers don't publish a cache-write rate.
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct Limit {
    pub context: u64,
    pub output: u64,
    #[serde(default)]
    pub input: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

const PER_MILLION_TO_PER_TOKEN: f64 = 1.0 / 1_000_000.0;

/// Normalize a rig `Usage` to our canonical convention — `input_tokens` is
/// the FRESH input only (cache reads are additive in `cached_input_tokens`).
///
/// rig fills `usage.input_tokens` inconsistently across providers:
/// - DeepSeek / OpenAI / Gemini / Groq / xAI / Mistral / etc. — `prompt_tokens`
///   from the OpenAI-shaped response, which is **total** prompt tokens (cached
///   is a labelled subset of this number). We subtract the cached portion to
///   get the fresh count.
/// - Anthropic — `input_tokens` from Anthropic's response, which excludes
///   cache reads already. No adjustment needed.
fn normalize_usage(provider: &str, u: &Usage) -> Usage {
    match provider {
        "anthropic" => *u,
        _ => Usage {
            input_tokens: u.input_tokens.saturating_sub(u.cached_input_tokens),
            ..*u
        },
    }
}

impl ModelEntry {
    /// **Convention:** `usage.input_tokens` is the FRESH input only;
    /// `cached_input_tokens` is additive. Callers must normalize first —
    /// see `ModelCatalogStore::compute`, which handles per-provider rig
    /// inconsistencies.
    pub fn cost_for(&self, u: &Usage) -> Option<f64> {
        let cost = self.cost.as_ref()?;
        let cache_read = cost.cache_read.unwrap_or(0.0);
        let total = (u.input_tokens as f64) * cost.input
            + (u.output_tokens as f64) * cost.output
            + (u.cached_input_tokens as f64) * cache_read;
        Some(total * PER_MILLION_TO_PER_TOKEN)
    }

    pub fn max_input_tokens(&self) -> Option<u64> {
        if self.limit.context == 0 {
            return None;
        }
        if let Some(input) = self.limit.input {
            return Some(input);
        }
        Some(self.limit.context.saturating_sub(self.limit.output))
    }

    /// Zero is the `Default::default()` sentinel — real models always
    /// publish a positive output limit, so we map zero to `None`.
    pub fn max_output_tokens(&self) -> Option<u64> {
        if self.limit.output == 0 {
            None
        } else {
            Some(self.limit.output)
        }
    }

    pub fn supports_function_calling(&self) -> bool {
        self.tool_call
    }

    pub fn supports_vision(&self) -> bool {
        self.attachment || self.modalities.input.iter().any(|s| s == "image")
    }

    pub fn supports_prompt_caching(&self) -> bool {
        self.cost
            .as_ref()
            .is_some_and(|c| c.cache_read.is_some() || c.cache_write.is_some())
    }

    pub fn supports_reasoning(&self) -> bool {
        self.reasoning
    }

    pub fn supports_response_schema(&self) -> bool {
        self.structured_output
    }

    /// Strict schema guarantees `input` / `output` are present whenever
    /// `cost` is, so the test reduces to `cost.is_some()`.
    pub fn has_pricing(&self) -> bool {
        self.cost.is_some()
    }
}

#[derive(Debug)]
pub struct ModelCatalogSnapshot {
    /// SHA-256 prefix (first 12 chars) of the source JSON bytes. Every row
    /// written under this version shares it.
    pub version: String,
    pub fetched_at: DateTime<Utc>,
    pub entries: HashMap<String, ModelEntry>,
}

impl ModelCatalogSnapshot {
    /// Reserved for tests. Production code uses `defaults()`.
    pub fn empty() -> Self {
        Self {
            version: "empty".to_string(),
            fetched_at: Utc::now(),
            entries: HashMap::new(),
        }
    }

    /// Hardcoded fallback for the no-cache first-boot path. Carries context
    /// windows + capability flags only — pricing is `None` until the
    /// scheduler's first refresh fills in live models.dev data.
    pub fn defaults() -> Self {
        let mut entries = HashMap::new();

        let claude = ModelEntry {
            limit: Limit { context: 200_000, output: 32_000, input: None },
            attachment: true,
            reasoning: true,
            tool_call: true,
            structured_output: true,
            ..Default::default()
        };
        for id in [
            "claude-opus-4-7",
            "claude-opus-4-8",
            "claude-opus-4-6",
            "claude-sonnet-4-5",
            "claude-sonnet-4-6",
            "claude-haiku-4-5",
            "claude-fable-5",
        ] {
            entries.insert(id.into(), claude.clone());
        }

        let gpt_4x = ModelEntry {
            limit: Limit { context: 128_000, output: 16_384, input: None },
            attachment: true,
            tool_call: true,
            structured_output: true,
            ..Default::default()
        };
        for id in ["gpt-4o", "gpt-4.1", "gpt-4.5"] {
            entries.insert(id.into(), gpt_4x.clone());
        }

        let o_series = ModelEntry {
            limit: Limit { context: 200_000, output: 65_536, input: None },
            tool_call: true,
            reasoning: true,
            structured_output: true,
            ..Default::default()
        };
        for id in ["o1", "o3", "o4"] {
            entries.insert(id.into(), o_series.clone());
        }

        let gemini_long = ModelEntry {
            limit: Limit { context: 1_000_000, output: 8_192, input: None },
            attachment: true,
            tool_call: true,
            structured_output: true,
            ..Default::default()
        };
        for id in [
            "gemini-2.0-flash",
            "gemini-2.5-pro",
            "gemini-2.5-flash",
            "gemini-1.5-pro",
        ] {
            entries.insert(id.into(), gemini_long.clone());
        }

        entries.insert(
            "deepseek-chat".into(),
            ModelEntry {
                limit: Limit { context: 64_000, output: 8_192, input: None },
                tool_call: true,
                structured_output: true,
                ..Default::default()
            },
        );
        entries.insert(
            "deepseek-reasoner".into(),
            ModelEntry {
                limit: Limit { context: 64_000, output: 8_192, input: None },
                tool_call: true,
                reasoning: true,
                ..Default::default()
            },
        );
        let dsv4 = ModelEntry {
            limit: Limit { context: 1_000_000, output: 8_192, input: None },
            tool_call: true,
            structured_output: true,
            ..Default::default()
        };
        entries.insert("deepseek-v4-pro".into(), dsv4.clone());
        entries.insert("deepseek-v4-flash".into(), dsv4);

        for id in [
            "llama-3.3-70b-versatile",
            "llama-3.1-70b",
            "llama-3.1-405b",
        ] {
            entries.insert(
                id.into(),
                ModelEntry {
                    limit: Limit { context: 128_000, output: 8_192, input: None },
                    tool_call: true,
                    ..Default::default()
                },
            );
        }

        entries.insert(
            "grok-2-latest".into(),
            ModelEntry {
                limit: Limit { context: 131_072, output: 8_192, input: None },
                tool_call: true,
                ..Default::default()
            },
        );

        entries.insert(
            "mistral-large-latest".into(),
            ModelEntry {
                limit: Limit { context: 128_000, output: 8_192, input: None },
                tool_call: true,
                structured_output: true,
                ..Default::default()
            },
        );

        entries.insert(
            "command-r-plus".into(),
            ModelEntry {
                limit: Limit { context: 128_000, output: 4_096, input: None },
                tool_call: true,
                ..Default::default()
            },
        );

        entries.insert(
            "qwen3-vl:32b".into(),
            ModelEntry {
                limit: Limit { context: 128_000, output: 8_192, input: None },
                attachment: true,
                tool_call: true,
                ..Default::default()
            },
        );

        Self {
            version: "defaults".to_string(),
            fetched_at: Utc::now(),
            entries,
        }
    }

    /// Resolution order:
    /// 1. `"{provider}/{model_id}"` — the canonical key shape for the fetched
    ///    models.dev catalog.
    /// 2. Bare `model_id` — fallback for the hardcoded `defaults()` snapshot
    ///    used pre-fetch, where entries are keyed by bare model id only.
    pub fn lookup(&self, m: &ModelRef) -> Option<&ModelEntry> {
        let composite = format!("{}/{}", m.provider, m.model_id);
        if let Some(p) = self.entries.get(&composite) {
            return Some(p);
        }
        self.entries.get(&m.model_id)
    }

    /// Like `lookup` but falls back to a longest-prefix walk so dated-suffix
    /// ids returned by provider APIs (e.g. `claude-opus-4-7-20250708`) still
    /// resolve to their family entry. When the model_id itself contains a
    /// vendor prefix (`qwen/qwen3.6-flash` from OpenRouter, Together AI etc.),
    /// the walk re-scopes to that vendor so provider-of-providers IDs resolve
    /// against the underlying vendor's catalog section.
    pub fn lookup_prefix(&self, provider: &str, model_id: &str) -> Option<&ModelEntry> {
        let mref = ModelRef {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            additional_params: None,
        };
        if let Some(e) = self.lookup(&mref) {
            return Some(e);
        }

        if let Some((vendor, rest)) = model_id.split_once('/') {
            let mref = ModelRef {
                provider: vendor.to_string(),
                model_id: rest.to_string(),
                additional_params: None,
            };
            if let Some(e) = self.lookup(&mref) {
                return Some(e);
            }
            return self.prefix_walk(vendor, rest);
        }

        self.prefix_walk(provider, model_id)
    }

    fn prefix_walk(&self, provider: &str, model_id: &str) -> Option<&ModelEntry> {
        let provider_prefix = format!("{provider}/");
        let mut best: Option<(usize, &str)> = None;
        for key in self.entries.keys() {
            let normalized = if let Some(stripped) = key.strip_prefix(&provider_prefix) {
                stripped
            } else if !key.contains('/') {
                key.as_str()
            } else {
                continue;
            };
            if !normalized.is_empty()
                && model_id.starts_with(normalized)
                && best.is_none_or(|(len, _)| normalized.len() > len)
            {
                best = Some((normalized.len(), key.as_str()));
            }
        }
        best.and_then(|(_, key)| self.entries.get(key))
    }
}

/// Hot-swappable wrapper around `Arc<ModelCatalogSnapshot>`. Readers call
/// `current()` for a cheap `Arc<…>`; the scheduler `swap()`s atomically.
/// Internal `Arc`s make clones cheap — `AppState` holds a bare
/// `ModelCatalogStore`, not an `Arc<ModelCatalogStore>`.
#[derive(Clone)]
pub struct ModelCatalogStore {
    inner: Arc<ArcSwap<ModelCatalogSnapshot>>,
    last_refresh_unix: Arc<AtomicI64>,
}

impl ModelCatalogStore {
    pub fn new(initial: ModelCatalogSnapshot) -> Self {
        Self {
            inner: Arc::new(ArcSwap::new(Arc::new(initial))),
            last_refresh_unix: Arc::new(AtomicI64::new(Utc::now().timestamp())),
        }
    }

    pub fn current(&self) -> Arc<ModelCatalogSnapshot> {
        self.inner.load_full()
    }

    pub fn compute(&self, m: &ModelRef, u: &Usage) -> (Option<f64>, String) {
        let p = self.current();
        let normalized = normalize_usage(&m.provider, u);
        (
            p.lookup(m).and_then(|e| e.cost_for(&normalized)),
            p.version.clone(),
        )
    }

    pub fn swap(&self, next: ModelCatalogSnapshot) {
        self.inner.store(Arc::new(next));
        self.last_refresh_unix.store(Utc::now().timestamp(), Ordering::Relaxed);
    }

    pub fn seconds_since_refresh(&self) -> i64 {
        (Utc::now().timestamp() - self.last_refresh_unix.load(Ordering::Relaxed)).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_prefix_matches_dated_suffix_against_bare_key() {
        let snap = ModelCatalogSnapshot::defaults();
        let entry = snap
            .lookup_prefix("anthropic", "claude-opus-4-7-20251210")
            .expect("dated suffix should fall back to bare prefix");
        assert_eq!(entry.limit.context, 200_000);
    }

    #[test]
    fn lookup_prefix_prefers_longest_match() {
        let mut entries = HashMap::new();
        entries.insert(
            "openai/gpt-4o".into(),
            ModelEntry { limit: Limit { context: 128_000, output: 16_384, input: None }, ..Default::default() },
        );
        entries.insert(
            "openai/gpt-4o-mini".into(),
            ModelEntry { limit: Limit { context: 128_000, output: 32_768, input: None }, ..Default::default() },
        );
        let snap = ModelCatalogSnapshot {
            version: "test".into(),
            fetched_at: Utc::now(),
            entries,
        };
        let entry = snap
            .lookup_prefix("openai", "gpt-4o-mini-2024-07-18")
            .expect("longest prefix should win");
        assert_eq!(entry.limit.output, 32_768);
    }

    #[test]
    fn lookup_prefix_resolves_openrouter_vendor_namespace() {
        let mut entries = HashMap::new();
        entries.insert(
            "qwen/qwen3-coder".into(),
            ModelEntry { limit: Limit { context: 256_000, output: 65_536, input: None }, ..Default::default() },
        );
        let snap = ModelCatalogSnapshot {
            version: "test".into(),
            fetched_at: Utc::now(),
            entries,
        };
        let entry = snap
            .lookup_prefix("openrouter", "qwen/qwen3-coder-plus")
            .expect("vendor-namespaced openrouter id should resolve");
        assert_eq!(entry.limit.context, 256_000);
    }

    #[test]
    fn lookup_prefix_does_not_cross_providers() {
        let mut entries = HashMap::new();
        entries.insert(
            "openai/gpt-4o".into(),
            ModelEntry { limit: Limit { context: 128_000, output: 16_384, input: None }, ..Default::default() },
        );
        let snap = ModelCatalogSnapshot {
            version: "test".into(),
            fetched_at: Utc::now(),
            entries,
        };
        assert!(snap.lookup_prefix("anthropic", "gpt-4o-mini").is_none());
    }
}
