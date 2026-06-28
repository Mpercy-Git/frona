use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::core::state::AppState;
use crate::notification::push_model::PushSubscription;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/push/vapid-public-key", get(get_vapid_public_key))
        .route("/api/push/subscribe", post(subscribe))
        .route("/api/push/unsubscribe", post(unsubscribe))
}

#[derive(Serialize)]
struct VapidPublicKeyResponse {
    public_key: Option<String>,
}

async fn get_vapid_public_key(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<VapidPublicKeyResponse>, ApiError> {
    Ok(Json(VapidPublicKeyResponse {
        public_key: state.config.push.vapid_public_key.clone(),
    }))
}

#[derive(Deserialize)]
struct SubscribeRequest {
    endpoint: String,
    expiration_time: Option<i64>,
    keys: SubscriptionKeys,
}

#[derive(Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

async fn subscribe(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SubscribeRequest>,
) -> Result<(), ApiError> {
    // Dedup: if a subscription with this endpoint already exists, delete it first.
    if state
        .push_subscription_repo
        .find_by_endpoint(&auth.user_id, &req.endpoint)
        .await?
        .is_some()
    {
        state
            .push_subscription_repo
            .delete_by_endpoint(&auth.user_id, &req.endpoint)
            .await?;
    }

    let sub = PushSubscription {
        id: crate::core::repository::new_id(),
        user_id: auth.user_id.clone(),
        endpoint: req.endpoint,
        expiration_time: req.expiration_time,
        p256dh_key: req.keys.p256dh,
        auth_secret: req.keys.auth,
        created_at: chrono::Utc::now(),
    };

    state.push_subscription_repo.create(&sub).await?;
    Ok(())
}

#[derive(Deserialize)]
struct UnsubscribeRequest {
    endpoint: String,
}

async fn unsubscribe(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UnsubscribeRequest>,
) -> Result<(), ApiError> {
    state
        .push_subscription_repo
        .delete_by_endpoint(&auth.user_id, &req.endpoint)
        .await?;
    Ok(())
}