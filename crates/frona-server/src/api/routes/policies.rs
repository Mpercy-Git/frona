use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::core::state::AppState;
use crate::policy::models::{CreatePolicyRequest, PolicyResponse, UpdatePolicyRequest};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/policies", get(list_policies).post(create_policy))
        .route(
            "/api/policies/{id}",
            get(get_policy).put(update_policy).delete(delete_policy),
        )
        .route("/api/policies/validate", axum::routing::post(validate_policy))
}

async fn list_policies(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<PolicyResponse>>, ApiError> {
    let policies = state.policy_service.list_policies(&auth.user_id).await?;
    let responses: Vec<PolicyResponse> = policies.into_iter().map(Into::into).collect();
    Ok(Json(responses))
}

async fn create_policy(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePolicyRequest>,
) -> Result<Json<PolicyResponse>, ApiError> {
    let policy = state
        .policy_service
        .create_policy(&auth.user_id, &req.policy_text)
        .await?;
    Ok(Json(policy.into()))
}

async fn get_policy(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PolicyResponse>, ApiError> {
    let policy = state.policy_service.get_policy(&auth.user_id, &id).await?;
    Ok(Json(policy.into()))
}

async fn update_policy(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePolicyRequest>,
) -> Result<Json<PolicyResponse>, ApiError> {
    let policy = state
        .policy_service
        .update_policy(&auth.user_id, &id, &req.policy_text)
        .await?;
    Ok(Json(policy.into()))
}

async fn delete_policy(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), ApiError> {
    state
        .policy_service
        .delete_policy(&auth.user_id, &id)
        .await?;
    Ok(())
}

#[derive(Deserialize)]
struct ValidateRequest {
    policy_text: String,
}

#[derive(serde::Serialize)]
struct ValidateResponse {
    valid: bool,
    error: Option<String>,
}

async fn validate_policy(
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ValidateRequest>,
) -> Json<ValidateResponse> {
    match state.policy_service.validate_policy_text(&req.policy_text) {
        Ok(()) => Json(ValidateResponse {
            valid: true,
            error: None,
        }),
        Err(e) => Json(ValidateResponse {
            valid: false,
            error: Some(e.to_string()),
        }),
    }
}
