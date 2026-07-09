use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::core::state::AppState;
use super::super::error::ApiError;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/.well-known/openid-configuration", get(openid_configuration))
        .route("/.well-known/jwks.json", get(jwks))
}

async fn openid_configuration(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let issuer = &state.config.server.issuer_url;
    Json(serde_json::json!({
        "issuer": issuer,
        "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
        "token_endpoint": format!("{issuer}/api/auth/tokens"),
        "response_types_supported": ["token"],
        "grant_types_supported": ["client_credentials"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["EdDSA"],
        "token_endpoint_auth_methods_supported": ["bearer"],
    }))
}

async fn jwks(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let keys = state.keypair_service.list_jwks().await?;
    Ok(Json(serde_json::json!({ "keys": keys })))
}
