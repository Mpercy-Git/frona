use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::core::config::{Config, SaveResult, redact_config_for_api};
use crate::core::state::AppState;
use crate::policy::models::PolicyAction;

use super::super::error::ApiError;
use super::super::middleware::auth::{AdminUser, AuthUser};

/// Server configuration is operator territory, reading it included — even
/// redacted, it names every configured provider, model group and billing
/// term. Gated on the same `list_users` capability the log stream uses to
/// mean "this person operates the server" (rather than a blanket admin-group
/// check) so a deployment's own policies stay in control of who that is. The
/// first registered user is promoted to `admins` by `ensure_admin_invariant`
/// during registration, so the setup wizard still works on a fresh install.
async fn require_operator(state: &AppState, auth: &AuthUser) -> Result<(), ApiError> {
    let caller = state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| {
            ApiError(crate::core::error::AppError::NotFound(
                "User not found".into(),
            ))
        })?;
    let decision = state
        .policy_service
        .authorize_user(&caller, PolicyAction::ListUsers)
        .await?;
    if decision.allowed {
        Ok(())
    } else {
        Err(ApiError(crate::core::error::AppError::Forbidden(
            "Changing server configuration requires administrator privileges".into(),
        )))
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/config/schema", get(get_schema))
        .route("/api/config", get(get_config).put(update_config))
        .route(
            "/api/config/environment-variables",
            get(list_environment_variables),
        )
}

/// Names likely to hold a secret, by a keyword in the variable's own name -
/// heuristic, not a schema, since the server has no registry of every
/// integration's env var names. Lets the settings UI offer `${VAR}`
/// references for provider credentials without ever exposing values.
const ENV_VAR_SECRET_KEYWORDS: &[&str] = &["API", "KEY", "TOKEN", "SECRET", "CREDENTIAL"];

async fn list_environment_variables(_admin: AdminUser) -> Json<Vec<String>> {
    let mut names: Vec<String> = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| {
            let upper = name.to_ascii_uppercase();
            ENV_VAR_SECRET_KEYWORDS
                .iter()
                .any(|keyword| upper.contains(keyword))
        })
        .collect();
    names.sort_unstable();
    Json(names)
}

/// The response shape for both direct config saves (`PUT /api/config`) and the
/// provider-admin mutation routes (`api/routes/providers.rs`), which also save
/// through `ConfigService`.
pub(super) fn response(mut result: SaveResult) -> Result<Json<Value>, ApiError> {
    redact_config_for_api(&mut result.config);
    Ok(Json(serde_json::json!({
        "config": result.config,
        "persisted_revision": result.persisted_revision,
        "active_revision": result.active_revision,
        "restart_required": result.restart_required,
    })))
}

async fn get_schema(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_operator(&state, &auth).await?;
    let schema = schemars::schema_for!(Config);
    Ok(Json(serde_json::to_value(schema).unwrap_or_default()))
}

/// A config file the server cannot read is the operator's to fix, not a bug
/// in the request - so it answers 422 with the loader's own message (which
/// names the file and, for the common mistakes, what to write) rather than a
/// 500 the client renders as a bare "Server error".
fn unreadable_config(error: crate::core::error::AppError) -> ApiError {
    let message = error.to_string();
    tracing::error!("{message}");
    ApiError(crate::core::error::AppError::Http {
        status: 422,
        message,
    })
}

async fn get_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    require_operator(&state, &auth).await?;
    // Rebuilt live from disk + FRONA_* env overrides on every request, the
    // same way the process does at startup - not `config_service.active()`
    // alone, which is a snapshot from server startup and won't reflect an
    // out-of-band edit (or a now-unreadable file) until a restart reloads it.
    let loaded = crate::core::config::ConfigService::load(crate::core::config::config_file_path())
        .map_err(unreadable_config)?;
    let mut value = serde_json::to_value(&loaded.config)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e.to_string())))?;
    redact_config_for_api(&mut value);
    // Carries the same `persisted_revision`/`active_revision`/`restart_required`
    // triple as `response()` below, so the provider-connections UI (whose
    // edit/delete routes require an `expected_persisted_revision`) can source
    // it from the initial load rather than needing a prior save first.
    let persisted_revision = loaded.revision().to_owned();
    let active_revision = state.config_service.active_revision().to_owned();
    let restart_required = persisted_revision != active_revision;
    Ok(Json(serde_json::json!({
        "config": value,
        "persisted_revision": persisted_revision,
        "active_revision": active_revision,
        "restart_required": restart_required,
    })))
}

#[derive(Deserialize)]
struct UpdateConfigRequest {
    patch: Value,
    #[serde(default)]
    expected_persisted_revision: Option<String>,
}

async fn update_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateConfigRequest>,
) -> Result<Json<Value>, ApiError> {
    require_operator(&state, &auth).await?;

    let result = state
        .config_service
        .save(body.patch, body.expected_persisted_revision.as_deref())
        .await?;

    state.set_runtime_config("setup_completed", "true").await?;

    response(result)
}
