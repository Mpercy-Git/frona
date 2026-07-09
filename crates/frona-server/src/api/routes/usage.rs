use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::db::repo::generic::SurrealRepo;
use crate::db::repo::tool_calls::ToolCallRepository;
use crate::inference::tool_call::ToolCall;
use crate::inference::usage::{
    BucketLatencyRow, ChatCostRow, InferenceUsage, InferenceUsageRepository, LatencyPercentiles,
    ModelLatencyRow, TimeBucket, UsageBucket, UsageRollup,
};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/chats/{id}/usage", get(chat_usage))
        .route("/api/users/{id}/usage", get(user_usage))
}

#[derive(Deserialize)]
struct UsageWindow {
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    /// `hour` | `day` — when set, the user response includes a time-series.
    /// Omitted on `/api/chats/...` (chat-scoped rollups don't expose this
    /// today). The repo layer enums this so SurrealDB gets a literal.
    bucket: Option<String>,
    /// Cap on the top-chats list (default 10). Ignored when no `bucket` is
    /// present since we don't compute top-chats outside the dashboard call.
    top_chats: Option<usize>,
}

fn parse_bucket(s: Option<&str>) -> Option<TimeBucket> {
    match s {
        Some("hour") => Some(TimeBucket::Hour),
        Some("day") => Some(TimeBucket::Day),
        _ => None,
    }
}

#[derive(Serialize)]
struct ChatUsageResponse {
    totals: UsageRollup,
    by_kind: HashMap<String, UsageRollup>,
    by_model: HashMap<String, UsageRollup>,
    /// `input_tokens` of the most recent `Chat` / `ToolTurn` call in the
    /// chat. Drives "context used so far" on page reload; the chat store
    /// updates it from live SSE events afterwards. `None` when no main-chat
    /// call has been recorded for this chat yet.
    last_chat_input_tokens: Option<u64>,
    /// Total number of tool invocations across all messages in the chat.
    /// Counted from the `tool_call` table; the chat store increments on each
    /// live `tool_call` SSE event afterwards.
    total_tool_calls: u64,
}

#[derive(Serialize)]
struct UserUsageResponse {
    totals: UsageRollup,
    by_kind: HashMap<String, UsageRollup>,
    by_model: HashMap<String, UsageRollup>,
    /// Time-series for charting — present only when `?bucket=hour|day` is
    /// in the query string. Buckets are RFC3339 UTC timestamps.
    #[serde(skip_serializing_if = "Option::is_none")]
    series: Option<Vec<UsageBucket>>,
    /// Latency percentiles over the window. Present only with `?bucket=`.
    #[serde(skip_serializing_if = "Option::is_none")]
    latency: Option<LatencyPercentiles>,
    /// Latency percentiles per model_ref — computed server-side via
    /// `array::map` + `math::percentile` subqueries per group.
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_by_model: Option<Vec<ModelLatencyRow>>,
    /// Per-bucket latency percentiles for charting trend lines, server-side.
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_series: Option<Vec<BucketLatencyRow>>,
    /// Top chats by cost in the window. Present only with `?bucket=`.
    #[serde(skip_serializing_if = "Option::is_none")]
    top_chats: Option<Vec<ChatCostRow>>,
}


async fn chat_usage(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<String>,
    Query(window): Query<UsageWindow>,
) -> Result<Json<ChatUsageResponse>, ApiError> {
    let chat = state.chat_service.find_chat(&chat_id).await?
        .ok_or_else(|| AppError::NotFound("chat not found".into()))?;
    if chat.user_id != auth.user_id {
        return Err(AppError::Forbidden("not your chat".into()).into());
    }

    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(state.db.clone());
    let totals = repo.aggregate_by_chat(&chat_id, window.since, window.until).await?;
    // For per-kind/per-model on a single chat, scope is implicit — use the user
    // aggregations gated on chat scope at the repo layer for v1. We accept that
    // by_kind/by_model are user-scoped here; the chat-scoped versions can be
    // added if/when the UI demands them.
    let by_kind = repo
        .aggregate_by_kind(&auth.user_id, window.since, window.until)
        .await?;
    let by_model = repo
        .aggregate_by_model(&auth.user_id, window.since, window.until)
        .await?;

    let last_chat_input_tokens = repo.last_chat_input_tokens(&chat_id).await?;
    let tc_repo: SurrealRepo<ToolCall> = SurrealRepo::new(state.db.clone());
    let total_tool_calls = tc_repo.count_by_chat_id(&chat_id).await?;

    Ok(Json(ChatUsageResponse {
        totals,
        by_kind,
        by_model,
        last_chat_input_tokens,
        total_tool_calls,
    }))
}

async fn user_usage(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Query(window): Query<UsageWindow>,
) -> Result<Json<UserUsageResponse>, ApiError> {
    if user_id != auth.user_id {
        return Err(AppError::Forbidden("not your usage".into()).into());
    }

    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(state.db.clone());
    let totals = repo.aggregate_by_user(&user_id, window.since, window.until).await?;
    let by_kind = repo.aggregate_by_kind(&user_id, window.since, window.until).await?;
    let by_model = repo.aggregate_by_model(&user_id, window.since, window.until).await?;

    // Dashboard mode — opt in with `?bucket=hour|day` plus a since/until
    // window. Adds time-series + latency percentiles + top chats in one
    // round-trip so the page doesn't fan out N requests.
    let (series, latency, latency_by_model, latency_series, top_chats) =
        if let Some(bucket) = parse_bucket(window.bucket.as_deref()) {
            let since = window
                .since
                .ok_or_else(|| AppError::Validation("bucket query requires `since`".into()))?;
            let until = window.until.unwrap_or_else(Utc::now);
            let series = repo
                .aggregate_buckets_by_user(&user_id, since, until, bucket)
                .await?;
            let latency = repo
                .latency_percentiles_by_user(&user_id, Some(since), Some(until))
                .await?;
            let top = repo
                .top_chats_by_user(&user_id, Some(since), Some(until), window.top_chats.unwrap_or(10))
                .await?;
            // SQL-side per-group percentiles via `array::map` —
            // see InferenceUsageRepository::latency_by_model.
            let by_model_pct = repo.latency_by_model(&user_id, since, until).await?;
            let bucket_pcts = repo
                .latency_by_bucket(&user_id, since, until, bucket)
                .await?;
            (
                Some(series),
                Some(latency),
                Some(by_model_pct),
                Some(bucket_pcts),
                Some(top),
            )
        } else {
            (None, None, None, None, None)
        };

    Ok(Json(UserUsageResponse {
        totals,
        by_kind,
        by_model,
        series,
        latency,
        latency_by_model,
        latency_series,
        top_chats,
    }))
}
