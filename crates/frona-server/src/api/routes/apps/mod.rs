mod proxy;

use axum::extract::{Path, State};
use axum::routing::{any, get, post};
use axum::{Json, Router};

use crate::app::models::{App, AppResponse};
use crate::core::error::AppError;
use crate::core::state::AppState;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps", get(list_apps))
        .route("/api/apps/{handle}", get(get_app).delete(delete_app))
        .route("/api/apps/{handle}/stop", post(stop_app))
        .route("/api/apps/{handle}/restart", post(restart_app))
        .route("/api/auth/apps", get(proxy::auth_gate))
        .route("/apps/{handle}", any(proxy::proxy_app_root))
        .route("/apps/{handle}/", any(proxy::proxy_app_root))
        .route("/apps/{handle}/{*path}", any(proxy::proxy_app_path))
}

/// 400 for malformed handles, 404 for missing or cross-user.
async fn resolve_user_app(state: &AppState, auth: &AuthUser, handle: &str) -> Result<App, ApiError> {
    let handle = crate::core::Handle::try_new(handle)?;
    let app = state
        .app_service
        .find_by_user_handle(&auth.user_id, &handle)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound(format!("App '{}' not found", handle.as_str()))))?;
    Ok(app)
}

async fn list_apps(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AppResponse>>, ApiError> {
    let apps = state.app_service.list_by_user(&auth.user_id).await?;
    Ok(Json(apps))
}

async fn get_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<AppResponse>, ApiError> {
    let app = resolve_user_app(&state, &auth, &handle).await?;
    Ok(Json(app.into()))
}

async fn delete_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<(), ApiError> {
    let app = resolve_user_app(&state, &auth, &handle).await?;
    state.app_service.destroy(&app.agent_id, &app.id).await?;
    Ok(())
}

async fn stop_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<AppResponse>, ApiError> {
    let app = resolve_user_app(&state, &auth, &handle).await?;
    let resp = state.app_service.stop(&app.agent_id, &app.id, &app.chat_id).await?;
    Ok(Json(resp))
}

async fn restart_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<AppResponse>, ApiError> {
    let app = resolve_user_app(&state, &auth, &handle).await?;
    let resp = state.app_service.restart(&app.agent_id, &app.id, &app.chat_id).await?;
    Ok(Json(resp))
}

