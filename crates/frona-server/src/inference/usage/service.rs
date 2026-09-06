//! Single recording funnel: every successful provider call goes through
//! `record()`, which persists the row, emits Prometheus metrics, and
//! dispatches the SSE event.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use metrics::{counter, gauge, histogram};
use rig_core::completion::request::Usage;

use crate::chat::broadcast::BroadcastService;
use crate::core::config::ProviderBillingKind;
use crate::core::repository::{Repository, new_id};
use crate::db::repo::generic::SurrealRepo;
use crate::inference::metadata::{ModelCatalogStore, total_prompt_usage};
use crate::inference::provider::ModelRef;
use crate::inference::usage::UsageContext;

use super::models::InferenceUsage;

// Metric names - `_total` suffix on counters per Prometheus convention; no
// `_histogram` suffix on histograms (auto-generated `_bucket`/`_count`/`_sum`).
pub const INFERENCE_INPUT_TOKENS_TOTAL: &str = "frona_inference_input_tokens_total";
pub const INFERENCE_CACHED_INPUT_TOKENS_TOTAL: &str = "frona_inference_cached_input_tokens_total";
pub const INFERENCE_OUTPUT_TOKENS_TOTAL: &str = "frona_inference_output_tokens_total";
pub const INFERENCE_COST_USD_TOTAL: &str = "frona_inference_cost_usd_total";
pub const INFERENCE_CALLS_TOTAL: &str = "frona_inference_calls_total";
pub const INFERENCE_DURATION_MS: &str = "frona_inference_duration_ms";
pub const INFERENCE_TTFT_MS: &str = "frona_inference_ttft_ms";
pub const INFERENCE_OUTPUT_TOKENS_PER_SECOND: &str = "frona_inference_output_tokens_per_second";
pub const INFERENCE_RETRY_OVERHEAD_MS: &str = "frona_inference_retry_overhead_ms";
pub const INFERENCE_RETRIES_TOTAL: &str = "frona_inference_retries_total";
pub const MODEL_METADATA_LOOKUP_MISSES_TOTAL: &str = "frona_model_metadata_lookup_misses_total";
pub const MODEL_METADATA_ENTRIES: &str = "frona_model_metadata_entries";
pub const MODEL_METADATA_REFRESH_AGE_SECONDS: &str =
    "frona_model_metadata_refresh_seconds_since_last";

/// Latency reading captured by the retry layer for a single recorded call.
/// `retry_overhead_ms` + `retry_count` describe retries within the recorded
/// model only; cross-model fallback is captured by `fallback_index`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LatencyMetrics {
    pub duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub retry_overhead_ms: u64,
    pub retry_count: u32,
}

#[derive(Clone)]
pub struct UsageService {
    /// Cloned from `AppState.model_catalog`. `ModelCatalogStore` is internally
    /// `Arc`-wrapped, so all clones share the same underlying ArcSwap - the
    /// scheduler can `swap()` from outside this service and we observe it.
    catalog: ModelCatalogStore,
    repo: SurrealRepo<InferenceUsage>,
    broadcast: BroadcastService,
    /// Billing kind per provider name, snapshotted from config at boot (see
    /// `Config::provider_billing_kinds`). Providers absent from the map fall
    /// back to `ProviderBillingKind::default_for_provider`, which keeps a
    /// provider reached without an explicit config entry classified sensibly.
    billing_kinds: Arc<HashMap<String, ProviderBillingKind>>,
}

impl UsageService {
    pub fn new(
        catalog: ModelCatalogStore,
        repo: SurrealRepo<InferenceUsage>,
        broadcast: BroadcastService,
        billing_kinds: Arc<HashMap<String, ProviderBillingKind>>,
    ) -> Self {
        Self {
            catalog,
            repo,
            broadcast,
            billing_kinds,
        }
    }

    /// How the provider serving `provider_name` bills, per config at boot.
    fn billing_kind(&self, provider_name: &str) -> ProviderBillingKind {
        self.billing_kinds
            .get(provider_name)
            .copied()
            .unwrap_or_else(|| ProviderBillingKind::default_for_provider(provider_name))
    }

    /// The only way to record an inference call.
    pub async fn record(
        &self,
        usage_ctx: &UsageContext,
        model_ref: &ModelRef,
        usage: &Usage,
        fallback_index: u8,
        latency: LatencyMetrics,
    ) {
        let (cost_usd, pricing_version) = self.catalog.compute(model_ref, usage);
        if cost_usd.is_none() {
            counter!(MODEL_METADATA_LOOKUP_MISSES_TOTAL, "model_ref" => model_ref.as_str())
                .increment(1);
        }
        let pricing = PricingSnapshot {
            cost_usd,
            pricing_version,
            billing_kind: self.billing_kind(model_ref.provider_name()),
        };
        let row = build_row(usage_ctx, model_ref, usage, fallback_index, latency, pricing);

        // Persistence failure logs but never propagates - observability never
        // blocks the user's reply.
        if let Err(e) = self.repo.create(&row).await {
            tracing::warn!(
                error = %e,
                "inference_usage persist failed; metrics + event still emitted"
            );
        }
        emit_metrics(&row);
        self.dispatch_event(&row);
    }

    /// Whether the catalog knows this model accepts image input. `None` when
    /// the model isn't in the catalog (capability unknown) — callers should
    /// treat unknown conservatively rather than assume support either way.
    pub fn model_supports_vision(&self, model_ref: &ModelRef) -> Option<bool> {
        self.catalog
            .current()
            .lookup_prefix(model_ref.provider.name(), &model_ref.model_id)
            .map(|e| e.supports_vision())
    }

    /// Called periodically (or on scrape) to keep the refresh-age gauge fresh.
    pub fn touch_refresh_age_gauge(&self) {
        gauge!(MODEL_METADATA_REFRESH_AGE_SECONDS).set(self.catalog.seconds_since_refresh() as f64);
    }

    fn dispatch_event(&self, row: &InferenceUsage) {
        let Some(chat_id) = row.chat_id.as_ref() else {
            return;
        };
        self.broadcast
            .broadcast_usage_recorded(crate::chat::broadcast::UsageRecorded {
                chat_id: chat_id.clone(),
                user_id: row.user_id.clone(),
                agent_id: row.agent_id.clone(),
                message_id: row.message_id.clone(),
                kind_tag: row.kind_tag.clone(),
                model_ref: row.model_ref.clone(),
                input_tokens: row.input_tokens,
                cached_input_tokens: row.cached_input_tokens,
                output_tokens: row.output_tokens,
                cost_usd: row.cost_usd,
                duration_ms: row.duration_ms,
                ttft_ms: row.ttft_ms,
                output_tokens_per_second: row.output_tokens_per_second,
                retry_overhead_ms: row.retry_overhead_ms,
                retry_count: row.retry_count,
                fallback_index: row.fallback_index,
            });
    }
}

fn emit_metrics(row: &InferenceUsage) {
    let user_id = row.user_id.clone();
    let agent_id = row.agent_id.clone().unwrap_or_default();
    let fb = row.fallback_index.to_string();
    let provider = row.provider.clone();
    let model_id = row.model_id.clone();
    let model_group = row.model_group.clone();
    let kind_tag = row.kind_tag.clone();

    let token_labels = [
        ("provider", provider.clone()),
        ("model_id", model_id.clone()),
        ("model_group", model_group.clone()),
        ("user_id", user_id.clone()),
        ("agent_id", agent_id.clone()),
        ("kind_tag", kind_tag.clone()),
        ("fallback_index", fb.clone()),
    ];
    let cost_labels = [
        ("provider", provider.clone()),
        ("model_id", model_id.clone()),
        ("model_group", model_group.clone()),
        ("user_id", user_id),
        ("agent_id", agent_id),
        ("kind_tag", kind_tag.clone()),
    ];
    let latency_labels = [
        ("provider", provider),
        ("model_id", model_id),
        ("kind_tag", kind_tag),
    ];

    counter!(INFERENCE_INPUT_TOKENS_TOTAL, &token_labels).increment(row.input_tokens);
    counter!(INFERENCE_CACHED_INPUT_TOKENS_TOTAL, &token_labels).increment(row.cached_input_tokens);
    counter!(INFERENCE_OUTPUT_TOKENS_TOTAL, &token_labels).increment(row.output_tokens);
    counter!(INFERENCE_CALLS_TOTAL, &token_labels).increment(1);
    counter!(INFERENCE_RETRIES_TOTAL, &token_labels).increment(row.retry_count as u64);
    if let Some(cost) = row.cost_usd {
        // The `metrics` 0.24 counter API only accepts u64. Track cost in
        // micro-dollars (cost * 1_000_000) so we get 6-decimal precision
        // through Prometheus while preserving the integer-counter semantics.
        // Grafana queries multiply by 1e-6 to display dollars.
        let micros = (cost * 1_000_000.0).max(0.0) as u64;
        counter!(INFERENCE_COST_USD_TOTAL, &cost_labels).increment(micros);
    }
    histogram!(INFERENCE_DURATION_MS, &latency_labels).record(row.duration_ms as f64);
    if let Some(ttft) = row.ttft_ms {
        histogram!(INFERENCE_TTFT_MS, &latency_labels).record(ttft as f64);
    }
    if let Some(tps) = row.output_tokens_per_second {
        histogram!(INFERENCE_OUTPUT_TOKENS_PER_SECOND, &latency_labels).record(tps);
    }
    if row.retry_overhead_ms > 0 {
        histogram!(INFERENCE_RETRY_OVERHEAD_MS, &latency_labels)
            .record(row.retry_overhead_ms as f64);
    }
}

/// What was known about price when a row was written, kept together because
/// the three are only ever meaningful as a set: a `cost_usd` says nothing
/// without the catalogue version it came from and the billing regime it was
/// charged under.
struct PricingSnapshot {
    cost_usd: Option<f64>,
    pricing_version: String,
    billing_kind: ProviderBillingKind,
}

fn build_row(
    usage_ctx: &UsageContext,
    model_ref: &ModelRef,
    usage: &Usage,
    fallback_index: u8,
    latency: LatencyMetrics,
    pricing: PricingSnapshot,
) -> InferenceUsage {
    let kind = &usage_ctx.kind;
    let output_tokens_per_second = compute_output_tps(usage.output_tokens, latency);
    // Rows carry one convention regardless of which provider reported the
    // call: `input_tokens` is the whole prompt, `cached_input_tokens` a subset
    // of it. Persisting rig's per-provider shape verbatim meant an Anthropic
    // row (three disjoint figures) and an OpenAI-shaped one (a total plus
    // labelled subsets) disagreed about what the same column meant.
    let usage = &total_prompt_usage(model_ref.provider_name(), usage);
    InferenceUsage {
        id: new_id(),
        user_id: usage_ctx.user_id.clone(),
        agent_id: kind.agent_id().map(str::to_owned),
        chat_id: kind.chat_id().map(str::to_owned),
        space_id: kind.space_id().map(str::to_owned),
        message_id: kind.message_id().map(str::to_owned),
        turn_index: kind.turn_index(),
        kind_tag: kind.tag().to_owned(),
        model_group: usage_ctx.model_group.clone(),
        provider: model_ref.provider_name().to_string(),
        model_id: model_ref.model_id.clone(),
        model_ref: model_ref.as_str(),
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        fallback_index,
        duration_ms: latency.duration_ms,
        ttft_ms: latency.ttft_ms,
        output_tokens_per_second,
        retry_overhead_ms: latency.retry_overhead_ms,
        retry_count: latency.retry_count,
        cost_usd: pricing.cost_usd,
        pricing_version: pricing.pricing_version,
        billing_kind: pricing.billing_kind.as_str().to_string(),
        created_at: Utc::now(),
    }
}

/// `output_tokens / generation_seconds` where generation_seconds is the wall
/// time after the first token arrived. Returns `None` if there's no usable
/// signal - no output tokens, no TTFT (non-streaming path), or duration shorter
/// than TTFT (clock skew between captures).
fn compute_output_tps(output_tokens: u64, latency: LatencyMetrics) -> Option<f64> {
    if output_tokens == 0 {
        return None;
    }
    let ttft = latency.ttft_ms?;
    let gen_ms = latency.duration_ms.checked_sub(ttft)?;
    if gen_ms == 0 {
        return None;
    }
    Some(output_tokens as f64 * 1000.0 / gen_ms as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ProviderModel;
    use crate::inference::usage::InferenceKind;

    fn row_for(provider: &str, usage: &Usage) -> InferenceUsage {
        let ctx = UsageContext::new(
            InferenceKind::Text {
                agent_id: "agent".into(),
                chat_id: "chat".into(),
                message_id: "msg".into(),
            },
            "user",
            "primary",
        );
        let model_ref = ModelRef {
            model_id: "test-model".to_string(),
            provider: ProviderModel::from_name(provider),
        };
        build_row(
            &ctx,
            &model_ref,
            usage,
            0,
            LatencyMetrics::default(),
            PricingSnapshot {
                cost_usd: None,
                pricing_version: "test".to_string(),
                billing_kind: ProviderBillingKind::default_for_provider(provider),
            },
        )
    }

    /// Anthropic reports the three prompt figures as disjoint numbers. Written
    /// straight through, the row claimed a 200-token prompt for a call that
    /// actually sent 1000, and the dashboard's `input - cached` went negative
    /// (clamped to zero) while its cache ratio read 300%.
    #[test]
    fn an_anthropic_row_persists_the_whole_prompt() {
        let row = row_for(
            "anthropic",
            &Usage {
                input_tokens: 200,
                cached_input_tokens: 600,
                cache_creation_input_tokens: 200,
                ..Default::default()
            },
        );
        assert_eq!(row.input_tokens, 1000);
        assert_eq!(row.cached_input_tokens, 600);
        assert_eq!(row.input_tokens - row.cached_input_tokens, 400);
    }

    /// OpenAI-shaped providers (OpenRouter among them) already report the
    /// total, so the row is unchanged and summing the two columns no longer
    /// counts the cached portion twice.
    #[test]
    fn an_openai_shaped_row_is_already_in_the_persisted_shape() {
        let row = row_for(
            "openrouter",
            &Usage {
                input_tokens: 1000,
                cached_input_tokens: 600,
                cache_creation_input_tokens: 200,
                ..Default::default()
            },
        );
        assert_eq!(row.input_tokens, 1000);
        assert_eq!(row.cached_input_tokens, 600);
    }
}
