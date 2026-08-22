use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::core::state::AppState;
use crate::notification::models::{Notification, NotificationData, NotificationLevel};
use crate::notification::push_model::PushSubscription;
use crate::notification::push_sender::PushDeliveryReport;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/push/vapid-public-key", get(get_vapid_public_key))
        .route("/api/push/subscribe", post(subscribe))
        .route("/api/push/unsubscribe", post(unsubscribe))
        .route("/api/push/test", post(send_test))
}

#[derive(Serialize)]
struct VapidPublicKeyResponse {
    public_key: Option<String>,
    /// True only when the server can actually *send*. A public key alone is
    /// enough for a browser to subscribe, so the UI needs to know whether the
    /// private half is there too — otherwise it reports success for a device
    /// that will never receive anything.
    can_send: bool,
}

async fn get_vapid_public_key(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<VapidPublicKeyResponse>, ApiError> {
    Ok(Json(VapidPublicKeyResponse {
        public_key: state.config.push.vapid_public_key.clone(),
        can_send: state.push_sender.is_some(),
    }))
}

#[derive(Serialize)]
struct TestPushResponse {
    /// False when the server has no usable VAPID key pair — the device is
    /// subscribed but nothing can ever be sent to it.
    configured: bool,
    #[serde(flatten)]
    report: PushDeliveryReport,
}

/// Push a throwaway notification to every device the user has registered and
/// report exactly what each push service said.
///
/// Deliberately *not* persisted: this is a delivery probe, not something worth
/// keeping in the notification list. It exists because "no notification
/// appeared" has half a dozen causes that are indistinguishable from the
/// device — no subscription stored, no VAPID key, a rejected signature, or a
/// push that was delivered and then swallowed by the service worker. This
/// separates the last case from all the others.
async fn send_test(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<TestPushResponse>, ApiError> {
    let sender = match &state.push_sender {
        Some(s) => s,
        None => {
            return Ok(Json(TestPushResponse {
                configured: false,
                report: PushDeliveryReport::default(),
            }))
        }
    };

    let notification = Notification {
        id: crate::core::repository::new_id(),
        user_id: auth.user_id.clone(),
        data: NotificationData::System {},
        level: NotificationLevel::Info,
        title: "Frona test notification".to_string(),
        body: "If you can see this, push notifications are working on this device.".to_string(),
        read: false,
        created_at: chrono::Utc::now(),
    };

    let report = sender.deliver_to_user(&auth.user_id, &notification).await;
    Ok(Json(TestPushResponse {
        configured: true,
        report,
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

const MAX_SUBSCRIPTIONS_PER_USER: usize = 20;

async fn subscribe(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SubscribeRequest>,
) -> Result<(), ApiError> {
    // Validate endpoint is an https:// URL to prevent SSRF.
    let parsed = req.endpoint.parse::<axum::http::Uri>()
        .map_err(|_| ApiError(crate::core::error::AppError::Validation("Invalid endpoint URL".into())))?;
    if parsed.scheme_str() != Some("https") {
        return Err(ApiError(crate::core::error::AppError::Validation(
            "Push endpoint must use HTTPS".into(),
        )));
    }

    // Enforce per-user subscription cap to prevent fan-out DoS.
    let existing = state.push_subscription_repo.find_by_user_id(&auth.user_id).await?;
    if existing.len() >= MAX_SUBSCRIPTIONS_PER_USER
        && !existing.iter().any(|s| s.endpoint == req.endpoint)
    {
        return Err(ApiError(crate::core::error::AppError::Validation(
            "Maximum number of push subscriptions reached".into(),
        )));
    }

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