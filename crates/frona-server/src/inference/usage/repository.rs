use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use surrealdb::types::SurrealValue;

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::{InferenceUsage, UsageRollup};

#[async_trait]
pub trait InferenceUsageRepository: Repository<InferenceUsage> {
    async fn aggregate_by_chat(
        &self,
        chat_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<UsageRollup, AppError>;

    async fn aggregate_by_user(
        &self,
        user_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<UsageRollup, AppError>;

    async fn aggregate_by_agent(
        &self,
        agent_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<UsageRollup, AppError>;

    async fn aggregate_by_kind(
        &self,
        user_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<HashMap<String, UsageRollup>, AppError>;

    async fn aggregate_by_model(
        &self,
        user_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<HashMap<String, UsageRollup>, AppError>;

    /// `input_tokens` of the latest `Chat` / `ToolTurn` row in the chat —
    /// used to rehydrate "context used so far" after a page reload before
    /// the next live SSE `usage_recorded` event fires.
    async fn last_chat_input_tokens(&self, chat_id: &str) -> Result<Option<u64>, AppError>;

    async fn aggregate_buckets_by_user(
        &self,
        user_id: &str,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
        bucket: TimeBucket,
    ) -> Result<Vec<UsageBucket>, AppError>;

    /// p50/p95/p99 of `duration_ms` and `ttft_ms` for the window. `None` for
    /// `ttft_ms` percentiles when no streaming row exists in the window.
    async fn latency_percentiles_by_user(
        &self,
        user_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<LatencyPercentiles, AppError>;

    async fn top_chats_by_user(
        &self,
        user_id: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<ChatCostRow>, AppError>;

    async fn latency_by_model(
        &self,
        user_id: &str,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Vec<ModelLatencyRow>, AppError>;

    async fn latency_by_bucket(
        &self,
        user_id: &str,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
        bucket: TimeBucket,
    ) -> Result<Vec<BucketLatencyRow>, AppError>;

    // ---- Instance-wide rollups -------------------------------------------
    //
    // Every method above is scoped to one user, chat or agent, which is the
    // right default: usage is personal data. These are not. They exist so an
    // operator can see what the *server* costs, and so they are gated one
    // level up by `PolicyAction::ViewUsageAnalytics` rather than by an
    // ownership check here. Never call one from a user-facing route without
    // that gate.

    async fn aggregate_all(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<UsageRollup, AppError>;

    /// Grouped by `(provider, billing_kind)`, not provider alone: an operator
    /// who moved a provider onto a subscription part-way through the window
    /// genuinely has two cost regimes in it, and averaging them would hide the
    /// only thing worth seeing.
    async fn aggregate_by_provider_all(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<ProviderSpendRow>, AppError>;

    /// Grouped by `(model_ref, provider, model_group, billing_kind)`. The
    /// model group is part of the key because that is the unit a
    /// recommendation can actually act on — "switch `reasoning` to X", not
    /// "switch this model everywhere it appears".
    async fn aggregate_by_model_all(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<ModelSpendRow>, AppError>;

    async fn aggregate_by_model_group_all(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<HashMap<String, UsageRollup>, AppError>;

    async fn aggregate_by_kind_all(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<HashMap<String, UsageRollup>, AppError>;

    /// Spend league table, highest first. Returns opaque `user_id`s — the
    /// caller resolves them to handles only if it is going to display them.
    async fn top_users_by_cost(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<UserCostRow>, AppError>;

    /// Instance-wide time series, for the admin dashboard's spend-over-time
    /// chart. The per-user equivalent is `aggregate_buckets_by_user`.
    async fn aggregate_buckets_all(
        &self,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
        bucket: TimeBucket,
    ) -> Result<Vec<UsageBucket>, AppError>;
}

/// One provider's instance-wide spend under one billing regime.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct ProviderSpendRow {
    pub provider: String,
    /// `""` on rows written before the column existed; read it through
    /// `ProviderBillingKind::from_str_or_metered`.
    #[serde(default)]
    pub billing_kind: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub calls: u64,
}

/// One model's instance-wide spend, carrying everything the repricing path
/// needs to value the *observed* token mix against a candidate model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct ModelSpendRow {
    pub model_ref: String,
    pub provider: String,
    pub model_group: String,
    #[serde(default)]
    pub billing_kind: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub calls: u64,
    /// Mean, deliberately not a percentile. `math::percentile` is not an
    /// aggregator (see `latency_by_model`), so a p50 per group costs one
    /// subquery per model — affordable when a user's row set is already
    /// narrowed by `idx_iu_user_created`, not when the scan is instance-wide.
    /// A mean is enough to answer "would this swap make things slower?".
    #[serde(default)]
    pub duration_ms_mean: Option<f64>,
    /// Calls the catalogue had no price for. These contribute 0 to `cost_usd`,
    /// so a model with a pricing gap looks free rather than unmeasured — which
    /// is the more dangerous of the two failure modes and worth reporting.
    #[serde(default)]
    pub uncosted_calls: u64,
}

/// A user's share of instance-wide spend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct UserCostRow {
    pub user_id: String,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub calls: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct ModelLatencyRow {
    pub model_ref: String,
    pub duration_ms_p50: Option<f64>,
    pub duration_ms_p95: Option<f64>,
    pub duration_ms_p99: Option<f64>,
    pub ttft_ms_p50: Option<f64>,
    pub ttft_ms_p95: Option<f64>,
    pub ttft_ms_p99: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct BucketLatencyRow {
    pub bucket: DateTime<Utc>,
    pub duration_ms_p50: Option<f64>,
    pub duration_ms_p95: Option<f64>,
    pub duration_ms_p99: Option<f64>,
    pub ttft_ms_p50: Option<f64>,
    pub ttft_ms_p95: Option<f64>,
    pub ttft_ms_p99: Option<f64>,
}

/// Closed set so SurrealDB's `time::floor` always gets a literal it can
/// index against `idx_iu_user_created`.
#[derive(Debug, Clone, Copy)]
pub enum TimeBucket {
    Hour,
    Day,
}

impl TimeBucket {
    pub fn duration_literal(&self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "1d",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct UsageBucket {
    pub bucket: DateTime<Utc>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub calls: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LatencyPercentiles {
    pub duration_ms_p50: Option<f64>,
    pub duration_ms_p95: Option<f64>,
    pub duration_ms_p99: Option<f64>,
    pub ttft_ms_p50: Option<f64>,
    pub ttft_ms_p95: Option<f64>,
    pub ttft_ms_p99: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct ChatCostRow {
    pub chat_id: String,
    pub cost_usd: f64,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}
