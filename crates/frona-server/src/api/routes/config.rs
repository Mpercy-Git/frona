use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::core::config::{
    Config, config_file_path, deep_merge, persist_config, redact_config_for_api,
    try_build_effective_config,
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

/// A config file the server cannot read is the operator's to fix, not a bug in
/// the request — so it answers 422 with the loader's own message (which names
/// the file and, for the common mistakes, what to write) rather than a 500 the
/// client renders as a bare "Server error". It used to be a panic inside the
/// handler: the connection died mid-response and the settings page could say
/// nothing beyond "Failed to load configuration".
fn unreadable_config(err: String) -> ApiError {
    tracing::error!("{err}");
    ApiError(crate::core::error::AppError::Http {
        status: 422,
        message: err,
    })
}

/// Puts back what was on disk before a save that turned out to be unreadable.
/// Best effort: if even the restore fails there is nothing left to do but say
/// so loudly — the operator still has the error from the save itself.
fn restore_config_file(path: &str, previous: Option<&str>) {
    let restored = match previous {
        Some(yaml) => std::fs::write(path, yaml),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = restored {
        tracing::error!("Failed to restore {path} after a bad save: {e}");
    }
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
    let config = try_build_effective_config(raw_yaml.as_deref()).map_err(unreadable_config)?;
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

    let previous_yaml = std::fs::read_to_string(&path).ok();
    let raw_yaml = previous_yaml.clone().unwrap_or_default();
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
    //
    // Reading it back is also the only check that covers the write itself:
    // `base` was validated before `persist_config` stripped its defaults from
    // it, and a strip that takes a field the loader needs (it has happened —
    // the `provider` tag of a model group) leaves a file that saves fine and
    // then can't be read, bricking the settings page and the next startup. If
    // that happens the previous file goes back and the save is refused.
    let saved_yaml = std::fs::read_to_string(&path).ok();
    let effective = match try_build_effective_config(saved_yaml.as_deref()) {
        Ok(effective) => effective,
        Err(err) => {
            restore_config_file(&path, previous_yaml.as_deref());
            return Err(ApiError(crate::core::error::AppError::Validation(format!(
                "Saved configuration could not be read back, so the previous {path} was restored. \
                 Please report this: {err}"
            ))));
        }
    };
    let mut response = serde_json::to_value(&effective)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e.to_string())))?;
    redact_config_for_api(&mut response);

    Ok(Json(serde_json::json!({
        "config": response,
        "restart_required": true,
    })))
}
