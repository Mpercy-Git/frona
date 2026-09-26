//! Fork-specific overlay on top of the shared `frona_model_catalog` crate:
//! byteplus's vendor alias, the OpenAI-protocol-from-npm-label translation,
//! rig `Usage` normalization, the hardcoded no-cache-yet fallback catalog,
//! and cache-write-aware pricing (`frona_model_catalog::ModelEntry::cost_for`
//! doesn't price cache writes at all, which would silently under-report
//! Anthropic cost).
//!
//! Extension traits, not inherent impls, throughout: `ModelCatalogSnapshot`,
//! `ModelCatalogStore` and `ModelEntry` are foreign types now, so none of
//! this can be an inherent method. Where a trait method's name might
//! otherwise collide with an inherent method the crate already defines
//! (`lookup_prefix`, `cost_for`), it's renamed — an inherent method always
//! wins silently over a trait method of the same name, which would make the
//! fork-side behavior (the byteplus alias, cache-write pricing) never run.

use std::collections::HashMap;

pub use frona_model_catalog::ModelCatalogStore;
pub use frona_model_catalog::catalog::{Cost, Limit, Modalities, ModelCatalogSnapshot, ModelEntry};

use chrono::Utc;
use rig_core::completion::request::Usage;

use crate::core::config::OpenAiApi;
use crate::inference::provider::ModelRef;

const PER_MILLION_TO_PER_TOKEN: f64 = 1.0 / 1_000_000.0;

/// Normalize a rig `Usage` to our canonical convention - `input_tokens` is
/// the FRESH input only; cache reads (`cached_input_tokens`) and cache writes
/// (`cache_creation_input_tokens`) are additive alongside it, each billed at
/// its own rate.
///
/// rig fills `usage.input_tokens` inconsistently across providers:
/// - DeepSeek / OpenAI / Gemini / Groq / xAI / OpenRouter / etc. -
///   `prompt_tokens` from the OpenAI-shaped response, which is **total** prompt
///   tokens; both cache figures are labelled subsets of that number, so we
///   subtract them to get the fresh count.
/// - Anthropic - `input_tokens` from Anthropic's response, which excludes both
///   already. No adjustment needed.
fn normalize_usage(provider: &str, u: &Usage) -> Usage {
    match provider {
        "anthropic" => *u,
        _ => Usage {
            input_tokens: u
                .input_tokens
                .saturating_sub(u.cached_input_tokens)
                .saturating_sub(u.cache_creation_input_tokens),
            ..*u
        },
    }
}

/// The other half of the convention: `input_tokens` as the TOTAL prompt size,
/// with cache reads and writes as labelled subsets of it. This is the shape
/// `InferenceUsage` rows persist and the usage dashboard reads — it derives
/// fresh input as `input - cached` and the cache ratio as `cached / input`,
/// both of which need the total as the denominator.
///
/// Only Anthropic needs adjusting: its API reports the three figures as
/// disjoint numbers, so a row written straight from it under-reports the
/// prompt and skews any ratio taken against it. Every OpenAI-shaped provider
/// already reports the total.
///
/// Costing goes the other way — see [`normalize_usage`]. Both derive from the
/// same raw `Usage`, so they are alternative views of one call, never applied
/// on top of each other.
pub fn total_prompt_usage(provider: &str, u: &Usage) -> Usage {
    match provider {
        "anthropic" => Usage {
            input_tokens: u
                .input_tokens
                .saturating_add(u.cached_input_tokens)
                .saturating_add(u.cache_creation_input_tokens),
            ..*u
        },
        _ => *u,
    }
}

/// Map a frona provider name onto the models.dev section that actually
/// publishes its models.
///
/// BytePlus ModelArk and Volcengine Ark are the international and mainland
/// halves of one platform serving one model family, and upstream catalogues it
/// under `volcengine` only. Without the alias every BytePlus call records
/// `cost_usd: None` and pins the context window to the 128K default — which
/// fires compaction, and the extra summarisation call it costs, far earlier
/// than a Seed/Doubao model requires. Same failure the OpenRouter vendor-prefix
/// walk exists to prevent, one lookup earlier.
fn catalog_provider(provider: &str) -> &str {
    match provider {
        "byteplus" => "volcengine",
        other => other,
    }
}

/// Map a models.dev npm package label to the wire protocol it implies.
/// `frona_model_catalog`'s `protocol_defaults` carries the raw npm label
/// (upstream is provider-execution-agnostic); only this fork's OpenAI
/// adapter needs to act on it.
pub(crate) fn openai_api_from_npm(npm: &str) -> Option<OpenAiApi> {
    match npm {
        "@ai-sdk/openai" => Some(OpenAiApi::Responses),
        "@ai-sdk/openai-compatible" => Some(OpenAiApi::ChatCompletions),
        _ => None,
    }
}

pub trait CatalogLookup {
    /// Resolution order:
    /// 1. `"{provider}/{model_id}"` - the canonical key shape for the fetched
    ///    models.dev catalog.
    /// 2. Bare `model_id` - fallback for the hardcoded `defaults()` snapshot
    ///    used pre-fetch, where entries are keyed by bare model id only.
    ///
    /// Applies the byteplus→volcengine alias; the crate's own
    /// `lookup_for_provider` doesn't know about it.
    fn lookup_exact(&self, m: &ModelRef) -> Option<&ModelEntry>;

    /// Like `lookup_exact` but falls back to a longest-prefix walk so
    /// dated-suffix ids returned by provider APIs (e.g.
    /// `claude-opus-4-7-20250708`) still resolve to their family entry.
    /// Applies the byteplus alias; the crate's own `lookup_prefix` doesn't.
    ///
    /// Named `lookup_model`, not `lookup_prefix`: the crate already defines
    /// an inherent `lookup_prefix` with the same signature but without the
    /// alias, and an inherent method always wins over a trait method of the
    /// same name, so a same-named trait method here would silently never
    /// run - byteplus would go back to being unpriced.
    fn lookup_model(&self, provider: &str, model_id: &str) -> Option<&ModelEntry>;

    /// The OpenAI wire protocol (chat completions vs. responses) this
    /// model's npm label implies, if any.
    fn model_protocol_default(&self, provider: &str, model_id: &str) -> Option<OpenAiApi>;
}

impl CatalogLookup for ModelCatalogSnapshot {
    fn lookup_exact(&self, m: &ModelRef) -> Option<&ModelEntry> {
        self.lookup_for_provider(catalog_provider(m.provider_name()), &m.model_id)
    }

    fn lookup_model(&self, provider: &str, model_id: &str) -> Option<&ModelEntry> {
        self.lookup_prefix(catalog_provider(provider), model_id)
    }

    fn model_protocol_default(&self, provider: &str, model_id: &str) -> Option<OpenAiApi> {
        self.protocol_hint(catalog_provider(provider), model_id)
            .and_then(openai_api_from_npm)
    }
}

pub trait CostForUsage {
    /// **Convention:** `usage.input_tokens` is the FRESH input only;
    /// `cached_input_tokens` and `cache_creation_input_tokens` are additive
    /// and priced separately. Callers must normalize first - see
    /// `ModelCatalogStore::price`, which handles per-provider rig
    /// inconsistencies.
    ///
    /// Named `full_cost_for`, not `cost_for`: the crate's own inherent
    /// `cost_for` takes a `TokenUsage` with no cache-write field at all, so
    /// it never prices Anthropic's cache-write premium. An inherent method
    /// always wins over a trait method of the same name, so this can't be
    /// called `cost_for` without silently losing that pricing.
    fn full_cost_for(&self, u: &Usage) -> Option<f64>;
}

impl CostForUsage for ModelEntry {
    fn full_cost_for(&self, u: &Usage) -> Option<f64> {
        let cost = self.cost.as_ref()?;
        let cache_read = cost.cache_read.unwrap_or(0.0);
        // A cache write is a premium over fresh input (Anthropic bills 1.25x),
        // so a model that publishes no write rate is charged at the plain input
        // rate rather than free — the tokens were processed either way.
        let cache_write = cost.cache_write.unwrap_or(cost.input);
        let total = (u.input_tokens as f64) * cost.input
            + (u.output_tokens as f64) * cost.output
            + (u.cached_input_tokens as f64) * cache_read
            + (u.cache_creation_input_tokens as f64) * cache_write;
        Some(total * PER_MILLION_TO_PER_TOKEN)
    }
}

/// Hardcoded fallback for the no-cache first-boot path. Carries context
/// windows + capability flags only - pricing is `None` until the
/// scheduler's first refresh fills in live models.dev data.
pub fn defaults() -> ModelCatalogSnapshot {
    let mut entries = HashMap::new();

    let claude = ModelEntry {
        limit: Limit {
            context: 200_000,
            output: 32_000,
            input: None,
        },
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
        limit: Limit {
            context: 128_000,
            output: 16_384,
            input: None,
        },
        attachment: true,
        tool_call: true,
        structured_output: true,
        ..Default::default()
    };
    for id in ["gpt-4o", "gpt-4.1", "gpt-4.5"] {
        entries.insert(id.into(), gpt_4x.clone());
    }

    let o_series = ModelEntry {
        limit: Limit {
            context: 200_000,
            output: 65_536,
            input: None,
        },
        tool_call: true,
        reasoning: true,
        structured_output: true,
        ..Default::default()
    };
    for id in ["o1", "o3", "o4"] {
        entries.insert(id.into(), o_series.clone());
    }

    let gemini_long = ModelEntry {
        limit: Limit {
            context: 1_000_000,
            output: 8_192,
            input: None,
        },
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
            limit: Limit {
                context: 64_000,
                output: 8_192,
                input: None,
            },
            tool_call: true,
            structured_output: true,
            ..Default::default()
        },
    );
    entries.insert(
        "deepseek-reasoner".into(),
        ModelEntry {
            limit: Limit {
                context: 64_000,
                output: 8_192,
                input: None,
            },
            tool_call: true,
            reasoning: true,
            ..Default::default()
        },
    );
    let dsv4 = ModelEntry {
        limit: Limit {
            context: 1_000_000,
            output: 8_192,
            input: None,
        },
        tool_call: true,
        structured_output: true,
        ..Default::default()
    };
    entries.insert("deepseek-v4-pro".into(), dsv4.clone());
    entries.insert("deepseek-v4-flash".into(), dsv4);

    for id in ["llama-3.3-70b-versatile", "llama-3.1-70b", "llama-3.1-405b"] {
        entries.insert(
            id.into(),
            ModelEntry {
                limit: Limit {
                    context: 128_000,
                    output: 8_192,
                    input: None,
                },
                tool_call: true,
                ..Default::default()
            },
        );
    }

    entries.insert(
        "grok-2-latest".into(),
        ModelEntry {
            limit: Limit {
                context: 131_072,
                output: 8_192,
                input: None,
            },
            tool_call: true,
            ..Default::default()
        },
    );

    entries.insert(
        "mistral-large-latest".into(),
        ModelEntry {
            limit: Limit {
                context: 128_000,
                output: 8_192,
                input: None,
            },
            tool_call: true,
            structured_output: true,
            ..Default::default()
        },
    );

    entries.insert(
        "command-r-plus".into(),
        ModelEntry {
            limit: Limit {
                context: 128_000,
                output: 4_096,
                input: None,
            },
            tool_call: true,
            ..Default::default()
        },
    );

    entries.insert(
        "qwen3-vl:32b".into(),
        ModelEntry {
            limit: Limit {
                context: 128_000,
                output: 8_192,
                input: None,
            },
            attachment: true,
            tool_call: true,
            ..Default::default()
        },
    );

    ModelCatalogSnapshot {
        version: "defaults".to_string(),
        fetched_at: Utc::now(),
        entries,
        providers: HashMap::new(),
        protocol_defaults: HashMap::new(),
    }
}

pub trait StorePricing {
    /// Prices a call, resolving through `lookup_model` rather than an exact
    /// key. Aggregators address models by a vendor-prefixed id
    /// (`openrouter` + `anthropic/claude-sonnet-4.5`) and direct providers add
    /// dated suffixes, neither of which is a literal catalog key — an exact
    /// lookup misses both and silently records `cost_usd: None`.
    fn price(&self, m: &ModelRef, u: &Usage) -> (Option<f64>, String);
}

impl StorePricing for ModelCatalogStore {
    fn price(&self, m: &ModelRef, u: &Usage) -> (Option<f64>, String) {
        let p = self.current();
        let normalized = normalize_usage(m.provider_name(), u);
        (
            p.lookup_model(m.provider_name(), &m.model_id)
                .and_then(|e| e.full_cost_for(&normalized)),
            p.version.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ProviderModel;

    fn priced_snapshot() -> ModelCatalogSnapshot {
        let mut entries = HashMap::new();
        entries.insert(
            "anthropic/claude-sonnet-4-6".into(),
            ModelEntry {
                cost: Some(Cost {
                    input: 3.0,
                    output: 15.0,
                    cache_read: Some(0.3),
                    cache_write: Some(3.75),
                }),
                limit: Limit {
                    context: 200_000,
                    output: 64_000,
                    input: None,
                },
                ..Default::default()
            },
        );
        ModelCatalogSnapshot {
            version: "test".to_string(),
            fetched_at: Utc::now(),
            entries,
            providers: HashMap::new(),
            protocol_defaults: HashMap::new(),
        }
    }

    /// Upstream files BytePlus's models under `volcengine` — the two are the
    /// international and mainland halves of one platform. Without the alias a
    /// `byteplus/…` ref misses on both the composite key and the prefix walk,
    /// so cost comes back `None` and the window falls to the 128K default.
    #[test]
    fn byteplus_resolves_against_the_volcengine_catalog_section() {
        let mut entries = HashMap::new();
        entries.insert(
            "volcengine/doubao-seed-1-8".into(),
            ModelEntry {
                cost: Some(Cost {
                    input: 2.0,
                    output: 8.0,
                    cache_read: None,
                    cache_write: None,
                }),
                limit: Limit {
                    context: 256_000,
                    output: 32_000,
                    input: None,
                },
                ..Default::default()
            },
        );
        let snapshot = ModelCatalogSnapshot {
            version: "test".to_string(),
            fetched_at: Utc::now(),
            entries,
            providers: HashMap::new(),
            protocol_defaults: HashMap::new(),
        };

        // Exact id, and the dated-suffix form the API actually returns.
        for model_id in ["doubao-seed-1-8", "doubao-seed-1-8-251228"] {
            let entry = snapshot
                .lookup_model("byteplus", model_id)
                .unwrap_or_else(|| panic!("{model_id} should resolve"));
            assert_eq!(entry.limit.context, 256_000);
            assert_eq!(entry.max_input_tokens(), Some(224_000));
        }

        let store = ModelCatalogStore::new(snapshot);
        let model = ModelRef {
            model_id: "doubao-seed-1-8-251228".to_string(),
            provider: ProviderModel::from_name("byteplus"),
        };
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            ..Default::default()
        };
        let (cost, _) = store.price(&model, &usage);
        assert_eq!(cost, Some(2.0), "byteplus calls must be priced, not None");
    }

    /// `openrouter` + `anthropic/claude-sonnet-4-6` is not a literal catalog
    /// key, so the exact lookup `price` used to do missed and every
    /// OpenRouter call was recorded with no cost at all.
    #[test]
    fn price_prices_a_vendor_prefixed_aggregator_model_id() {
        let store = ModelCatalogStore::new(priced_snapshot());
        let model = ModelRef {
            model_id: "anthropic/claude-sonnet-4-6".to_string(),
            provider: ProviderModel::from_name("openrouter"),
        };
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            ..Default::default()
        };

        let (cost, _) = store.price(&model, &usage);
        assert_eq!(cost, Some(3.0));
    }

    /// OpenAI-shaped usage reports `prompt_tokens` as the total, with the
    /// cached and newly-written portions as labelled subsets. Billing all
    /// three at the full input rate would triple-count the same tokens.
    #[test]
    fn price_splits_openai_shaped_prompt_tokens_across_the_three_rates() {
        let store = ModelCatalogStore::new(priced_snapshot());
        let model = ModelRef {
            model_id: "anthropic/claude-sonnet-4-6".to_string(),
            provider: ProviderModel::from_name("openrouter"),
        };
        let usage = Usage {
            input_tokens: 1_000_000,
            cached_input_tokens: 600_000,
            cache_creation_input_tokens: 200_000,
            output_tokens: 0,
            ..Default::default()
        };

        // 200k fresh @ 3.00 + 600k read @ 0.30 + 200k write @ 3.75
        let (cost, _) = store.price(&model, &usage);
        let cost = cost.expect("priced");
        assert!(
            (cost - (0.6 + 0.18 + 0.75)).abs() < 1e-9,
            "unexpected cost {cost}"
        );
    }

    /// Anthropic's own response already excludes cache reads from
    /// `input_tokens`, so the subtraction must not be applied twice.
    #[test]
    fn price_leaves_anthropic_native_input_tokens_alone() {
        let store = ModelCatalogStore::new(priced_snapshot());
        let model = ModelRef {
            model_id: "claude-sonnet-4-6".to_string(),
            provider: ProviderModel::from_name("anthropic"),
        };
        let usage = Usage {
            input_tokens: 1_000_000,
            cached_input_tokens: 600_000,
            output_tokens: 0,
            ..Default::default()
        };

        // 1M fresh @ 3.00 + 600k read @ 0.30
        let (cost, _) = store.price(&model, &usage);
        let cost = cost.expect("priced");
        assert!((cost - (3.0 + 0.18)).abs() < 1e-9, "unexpected cost {cost}");
    }

    #[test]
    fn total_prompt_usage_restores_anthropics_disjoint_figures() {
        let usage = Usage {
            input_tokens: 200,
            cached_input_tokens: 600,
            cache_creation_input_tokens: 200,
            ..Default::default()
        };
        assert_eq!(total_prompt_usage("anthropic", &usage).input_tokens, 1000);
    }

    #[test]
    fn total_prompt_usage_leaves_openai_shaped_totals_alone() {
        let usage = Usage {
            input_tokens: 1000,
            cached_input_tokens: 600,
            cache_creation_input_tokens: 200,
            ..Default::default()
        };
        assert_eq!(total_prompt_usage("openrouter", &usage).input_tokens, 1000);
    }

    /// The two views are alternatives, not a pipeline: whichever provider
    /// reported the call, the persisted total minus the two cache figures is
    /// the fresh count that was priced at the full input rate.
    #[test]
    fn the_two_views_agree_on_the_fresh_count() {
        for (provider, raw) in [
            (
                "anthropic",
                Usage {
                    input_tokens: 200,
                    cached_input_tokens: 600,
                    cache_creation_input_tokens: 200,
                    ..Default::default()
                },
            ),
            (
                "openrouter",
                Usage {
                    input_tokens: 1000,
                    cached_input_tokens: 600,
                    cache_creation_input_tokens: 200,
                    ..Default::default()
                },
            ),
        ] {
            let total = total_prompt_usage(provider, &raw).input_tokens;
            let fresh = normalize_usage(provider, &raw).input_tokens;
            assert_eq!(
                total - raw.cached_input_tokens - raw.cache_creation_input_tokens,
                fresh,
                "{provider}"
            );
            assert_eq!(fresh, 200, "{provider}");
        }
    }

    /// A model with no published write rate still processed the tokens, so
    /// they are charged as fresh input rather than dropped on the floor.
    #[test]
    fn cost_for_charges_cache_writes_at_the_input_rate_when_none_is_published() {
        let entry = ModelEntry {
            cost: Some(Cost {
                input: 3.0,
                output: 15.0,
                cache_read: Some(0.3),
                cache_write: None,
            }),
            ..Default::default()
        };
        let usage = Usage {
            input_tokens: 0,
            cache_creation_input_tokens: 1_000_000,
            ..Default::default()
        };
        assert_eq!(entry.full_cost_for(&usage), Some(3.0));
    }

    #[test]
    fn lookup_prefix_matches_dated_suffix_against_bare_key() {
        let snap = defaults();
        let entry = snap
            .lookup_model("anthropic", "claude-opus-4-7-20251210")
            .expect("dated suffix should fall back to bare prefix");
        assert_eq!(entry.limit.context, 200_000);
    }

    #[test]
    fn lookup_prefix_prefers_longest_match() {
        let mut entries = HashMap::new();
        entries.insert(
            "openai/gpt-4o".into(),
            ModelEntry {
                limit: Limit {
                    context: 128_000,
                    output: 16_384,
                    input: None,
                },
                ..Default::default()
            },
        );
        entries.insert(
            "openai/gpt-4o-mini".into(),
            ModelEntry {
                limit: Limit {
                    context: 128_000,
                    output: 32_768,
                    input: None,
                },
                ..Default::default()
            },
        );
        let snap = ModelCatalogSnapshot {
            version: "test".into(),
            fetched_at: Utc::now(),
            entries,
            providers: HashMap::new(),
            protocol_defaults: HashMap::new(),
        };
        let entry = snap
            .lookup_model("openai", "gpt-4o-mini-2024-07-18")
            .expect("longest prefix should win");
        assert_eq!(entry.limit.output, 32_768);
    }

    #[test]
    fn lookup_prefix_resolves_openrouter_vendor_namespace() {
        let mut entries = HashMap::new();
        entries.insert(
            "qwen/qwen3-coder".into(),
            ModelEntry {
                limit: Limit {
                    context: 256_000,
                    output: 65_536,
                    input: None,
                },
                ..Default::default()
            },
        );
        let snap = ModelCatalogSnapshot {
            version: "test".into(),
            fetched_at: Utc::now(),
            entries,
            providers: HashMap::new(),
            protocol_defaults: HashMap::new(),
        };
        let entry = snap
            .lookup_model("openrouter", "qwen/qwen3-coder-plus")
            .expect("vendor-namespaced openrouter id should resolve");
        assert_eq!(entry.limit.context, 256_000);
    }

    #[test]
    fn lookup_prefix_does_not_cross_providers() {
        let mut entries = HashMap::new();
        entries.insert(
            "openai/gpt-4o".into(),
            ModelEntry {
                limit: Limit {
                    context: 128_000,
                    output: 16_384,
                    input: None,
                },
                ..Default::default()
            },
        );
        let snap = ModelCatalogSnapshot {
            version: "test".into(),
            fetched_at: Utc::now(),
            entries,
            providers: HashMap::new(),
            protocol_defaults: HashMap::new(),
        };
        assert!(snap.lookup_model("anthropic", "gpt-4o-mini").is_none());
    }
}
