use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::core::state::AppState;
use crate::notification::models::{Notification, NotificationPreferences};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/notifications", get(list_notifications))
        .route("/api/notifications/{id}/read", post(mark_read))
        .route("/api/notifications/read-all", post(mark_all_read))
        .route(
            "/api/notifications/preferences",
            get(get_preferences).put(put_preferences),
        )
}

#[derive(Serialize)]
struct NotificationsResponse {
    notifications: Vec<Notification>,
    unread_count: u64,
}

async fn list_notifications(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<NotificationsResponse>, ApiError> {
    let notifications = state.notification_service.list(&auth.user_id, 50).await?;
    let unread_count = state
        .notification_service
        .unread_count(&auth.user_id)
        .await?;

    Ok(Json(NotificationsResponse {
        notifications,
        unread_count,
    }))
}

async fn mark_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), ApiError> {
    state
        .notification_service
        .mark_read(&auth.user_id, &id)
        .await?;
    Ok(())
}

async fn mark_all_read(auth: AuthUser, State(state): State<AppState>) -> Result<(), ApiError> {
    state
        .notification_service
        .mark_all_read(&auth.user_id)
        .await?;
    Ok(())
}

async fn get_preferences(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Json<NotificationPreferences> {
    Json(state.notification_service.preferences(&auth.user_id).await)
}

async fn put_preferences(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(prefs): Json<NotificationPreferences>,
) -> Result<Json<NotificationPreferences>, ApiError> {
    let saved = state
        .notification_service
        .set_preferences(&auth.user_id, prefs)
        .await?;
    Ok(Json(saved))
}
