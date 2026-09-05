use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::core::config::{
    Config, build_effective_config, config_file_path, deep_merge, persist_config,
    redact_config_for_api,
};
use crate::core::state::AppState;
use crate::policy::models::PolicyAction;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

/// Server configuration is operator territory. `GET` stays open to any signed-in
/// user (it is redacted, and the settings UI reads it to render), but writing it
/// — provider credentials, billing terms, auth secrets — is gated on the same
/// `list_users` capability the log stream uses to mean "this person operates the
/// server". The first registered user is promoted to `admins` by
/// `ensure_admin_invariant` during registration, so the setup wizard still works
/// on a fresh install.
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
}

async fn get_schema(_auth: AuthUser) -> Json<serde_json::Value> {
    let schema = schemars::schema_for!(Config);
    Json(serde_json::to_value(schema).unwrap_or_default())
}

async fn get_config(_auth: AuthUser) -> Result<Json<serde_json::Value>, ApiError> {
    // Rebuild the effective config the same way the process does at startup
    // (disk YAML + FRONA_* env overrides), rather than reading disk alone
    // (state.config is a startup snapshot and is never mutated in place).
    // Otherwise a value pinned by an env var (e.g. FRONA_BROWSER_WS_URL) can
    // show — and test — differently in the settings UI than what's actually
    // running, since env vars always win at real startup.
    let path = config_file_path();
    let raw_yaml = std::fs::read_to_string(&path).ok();
    let config = build_effective_config(raw_yaml.as_deref());
    let mut value = serde_json::to_value(&config)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e.to_string())))?;
    redact_config_for_api(&mut value);
    Ok(Json(value))
}

async fn update_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(patch): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_operator(&state, &auth).await?;

    let path = config_file_path();

    let raw_yaml = std::fs::read_to_string(&path).unwrap_or_default();
    let mut base: serde_json::Value = if raw_yaml.is_empty() {
        serde_json::json!({})
    } else {
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(&raw_yaml).map_err(|e| {
            ApiError(crate::core::error::AppError::Internal(format!(
                "Failed to parse existing config.yaml: {e}"
            )))
        })?;
        serde_json::to_value(yaml_val).map_err(|e| {
            ApiError(crate::core::error::AppError::Internal(format!(
                "Failed to convert YAML to JSON: {e}"
            )))
        })?
    };

    deep_merge(&mut base, patch);

    let _: Config = serde_json::from_value(base.clone()).map_err(|e| {
        ApiError(crate::core::error::AppError::Validation(format!(
            "Invalid config: {e}"
        )))
    })?;

    // Strip defaults and persist to disk (mutates `base` in place).
    persist_config(&mut base, &path)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e)))?;

    state.set_runtime_config("setup_completed", "true").await?;

    // Build the API response from the effective config (disk + FRONA_* env
    // overrides), the same way GET /api/config does — not from `validated`
    // above, which reflects only what was just saved. A value the user just
    // typed here can still be shadowed by an env var at actual startup, and
    // the response should say so rather than echo back what won't take effect.
    let saved_yaml = std::fs::read_to_string(&path).ok();
    let effective = build_effective_config(saved_yaml.as_deref());
    let mut response = serde_json::to_value(&effective)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e.to_string())))?;
    redact_config_for_api(&mut response);

    Ok(Json(serde_json::json!({
        "config": response,
        "restart_required": true,
    })))
}
