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
    let mut value = serde_json::to_value(state.config.as_ref())
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

    let _: Config = serde_json::from_value(base.clone())
        .map_err(|e| ApiError(crate::core::error::AppError::Validation(
            format!("Invalid config: {e}"),
        )))?;

    persist_config(&mut base, &path)
        .map_err(|e| ApiError(crate::core::error::AppError::Internal(e)))?;

    state.set_runtime_config("setup_completed", "true").await?;

    redact_config_for_api(&mut base);

    Ok(Json(serde_json::json!({
        "config": base,
        "restart_required": true,
    })))
}
