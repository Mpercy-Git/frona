//! Admin cost surface: instance-wide usage rollups and the cost-report history.
//!
//! Every handler gates on `PolicyAction::ViewUsageAnalytics` before touching
//! the cost service, which performs no authorization of its own and reads every
//! user's rows.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::models::COST_ANALYST_AGENT_HANDLE;
use crate::agent::task::models::{CreateTaskRequest, TaskResponse};
use crate::auth::models::User;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::cost::models::CostReport;
use crate::cost::service::SpendAnalysis;
use crate::policy::models::PolicyAction;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

/// Default report page size. Reports are written monthly by default, so this
/// is several years of history.
const DEFAULT_REPORT_LIMIT: u32 = 50;
const MAX_REPORT_LIMIT: u32 = 200;
const DEFAULT_WINDOW_DAYS: i64 = 30;
const MAX_WINDOW_DAYS: i64 = 365;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/usage", get(admin_usage))
        .route("/api/admin/cost-reports", get(list_reports))
        .route("/api/admin/cost-reports/run", post(run_analysis))
        .route("/api/admin/cost-reports/{id}", get(get_report))
}

async fn load_caller(state: &AppState, auth: &AuthUser) -> Result<User, AppError> {
    state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))
}

/// The single gate for this module. Mirrors `routes::admin::require`.
async fn require_analytics(state: &AppState, caller: &User) -> Result<(), AppError> {
    let decision = state
        .policy_service
        .authorize_user(caller, PolicyAction::ViewUsageAnalytics)
        .await?;
    if decision.allowed {
        Ok(())
    } else {
        Err(AppError::Forbidden(if decision.diagnostics.is_empty() {
            "Not permitted".into()
        } else {
            decision.diagnostics
        }))
    }
}

#[derive(Deserialize)]
struct UsageWindow {
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    /// Convenience alternative to `since`/`until`, in days back from now.
    days: Option<i64>,
}

impl UsageWindow {
    /// Explicit bounds win; otherwise `days` back from now, clamped so an
    /// unbounded request can't scan the whole table.
    fn resolve(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        let until = self.until.unwrap_or_else(Utc::now);
        let since = self.since.unwrap_or_else(|| {
            until
                - chrono::Duration::days(
                    self.days
                        .unwrap_or(DEFAULT_WINDOW_DAYS)
                        .clamp(1, MAX_WINDOW_DAYS),
                )
        });
        (since, until)
    }
}

async fn admin_usage(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(window): Query<UsageWindow>,
) -> Result<Json<SpendAnalysis>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require_analytics(&state, &caller).await?;

    let (since, until) = window.resolve();
    Ok(Json(state.cost_service.analyse(since, until).await?))
}

#[derive(Deserialize)]
struct ListReportsQuery {
    limit: Option<u32>,
}

async fn list_reports(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ListReportsQuery>,
) -> Result<Json<Vec<CostReport>>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require_analytics(&state, &caller).await?;

    let limit = query
        .limit
        .unwrap_or(DEFAULT_REPORT_LIMIT)
        .clamp(1, MAX_REPORT_LIMIT);
    Ok(Json(state.cost_service.list_reports(limit).await?))
}

async fn get_report(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CostReport>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require_analytics(&state, &caller).await?;

    let report = state
        .cost_service
        .get_report(&id)
        .await?
        .ok_or_else(|| AppError::NotFound("Cost report not found".into()))?;
    Ok(Json(report))
}

#[derive(Serialize)]
struct RunAnalysisResponse {
    task: TaskResponse,
}

/// Queue an ad-hoc analysis. The task is created `Pending`; the scheduler's
/// next `run_pending_tasks` sweep executes it, exactly as it would a scheduled
/// run. Returns the task so the UI can follow it rather than blocking on an
/// LLM round-trip.
async fn run_analysis(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<RunAnalysisResponse>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require_analytics(&state, &caller).await?;

    // The caller's own copy of the built-in — reports are filed under whoever
    // asked for them. It exists for any user eligible for the agent, but a
    // permission granted by custom policy rather than group membership can
    // leave a user authorized here without having been provisioned one.
    let agent = state
        .agent_service
        .find_by_handle(&caller.id, COST_ANALYST_AGENT_HANDLE)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "No '{COST_ANALYST_AGENT_HANDLE}' agent exists for this account. It is provisioned \
                 for members of the 'admins' group."
            ))
        })?;

    let task = state
        .task_service
        .create(
            &caller.id,
            CreateTaskRequest {
                agent_id: agent.id,
                space_id: None,
                chat_id: None,
                title: "Cost analysis".to_string(),
                description: Some(
                    "Review this server's inference spend for the last 30 days and file a cost \
                     report. Start with list_provider_billing, then analyse_spend, then \
                     compare_models for any alternative you are considering, then \
                     save_cost_report."
                        .to_string(),
                ),
                source_agent_id: None,
                source_chat_id: None,
                resume_parent: None,
                run_at: None,
                quarantined: false,
                result_schema: None,
                result_description: None,
            },
        )
        .await?;

    Ok(Json(RunAnalysisResponse { task }))
}
