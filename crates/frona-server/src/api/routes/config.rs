use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::core::config::{
    Config, config_file_path, deep_merge, persist_config, redact_config_for_api,
};
use crate::core::state::AppState;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;


pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/config/schema", get(get_schema))
        .route("/api/config", get(get_config).put(update_config))
}

async fn get_schema(_auth: AuthUser) -> Json<serde_json::Value> {
    let schema = schemars::schema_for!(Config);
    Json(serde_json::to_value(schema).unwrap_or_default())
}

async fn get_config(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Always read from disk so the response reflects settings saved via PUT
    // (state.config is the snapshot at startup and is never mutated in place).
    let path = config_file_path();
    let raw_yaml = std::fs::read_to_string(&path).unwrap_or_default();
    let config: Config = if raw_yaml.is_empty() {
        (*state.config).clone()
    } else {
        let base: serde_json::Value = serde_yaml::from_str::<serde_yaml::Value>(&raw_yaml)
            .ok()
            .and_then(|y| serde_json::to_value(y).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        serde_json::from_value(base).unwrap_or_else(|_| (*state.config).clone())
    };
    let mut value = serde_json::to_value(&config)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e.to_string())))?;
    redact_config_for_api(&mut value);
    Ok(Json(value))
}

async fn update_config(
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(patch): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let path = config_file_path();

    let raw_yaml = std::fs::read_to_string(&path).unwrap_or_default();
    let mut base: serde_json::Value = if raw_yaml.is_empty() {
        serde_json::json!({})
    } else {
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(&raw_yaml)
            .map_err(|e| ApiError(crate::core::error::AppError::Internal(
                format!("Failed to parse existing config.yaml: {e}"),
            )))?;
        serde_json::to_value(yaml_val)
            .map_err(|e| ApiError(crate::core::error::AppError::Internal(
                format!("Failed to convert YAML to JSON: {e}"),
            )))?
    };

    deep_merge(&mut base, patch);

    // Validate the merged config and capture the fully-deserialised struct.
    // Deserialising into `Config` fills in all default values (via
    // `#[serde(default)]`), so the response we return to the frontend has
    // every field present — matching the shape of GET /api/config.
    let validated: Config = serde_json::from_value(base.clone())
        .map_err(|e| ApiError(crate::core::error::AppError::Validation(
            format!("Invalid config: {e}"),
        )))?;

    // Strip defaults and persist to disk (mutates `base` in place).
    persist_config(&mut base, &path)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e)))?;

    state.set_runtime_config("setup_completed", "true").await?;

    // Build the API response from the validated config (with all defaults
    // filled in), NOT from the stripped `base` that was written to disk.
    let mut response = serde_json::to_value(&validated)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e.to_string())))?;
    redact_config_for_api(&mut response);

    Ok(Json(serde_json::json!({
        "config": response,
        "restart_required": true,
    })))
}
