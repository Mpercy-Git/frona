mod proxy;

use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
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
        .route(proxy::APP_CALLBACK_PATH, get(proxy::auth_gate_callback))
        .route("/apps/{handle}", any(proxy::proxy_app_root))
        .route("/apps/{handle}/", any(proxy::proxy_app_root))
        .route("/apps/{handle}/{*path}", any(proxy::proxy_app_path))
}

/// Keeps agent-built apps off Frona's own origin when `server.apps_url` is set.
///
/// The apps origin serves `/apps/*` and the session handoff and nothing else:
/// no API, no frontend. Any other host sends `/apps/*` over to the apps
/// origin and refuses the handoff. The decision rests on the `Host` header,
/// which a browser will not let a page set, so the reverse proxy in front must
/// pass it through unchanged.
pub async fn apps_origin_guard(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let (Some(apps_host), Some(apps_url)) = (
        state.config.server.apps_host(),
        state.config.server.public_apps_url(),
    ) else {
        return next.run(request).await;
    };

    let host = request
        .uri()
        .authority()
        .map(|a| a.as_str())
        .or_else(|| {
            request
                .headers()
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = request.uri().path();
    let is_app_path = path.starts_with(proxy::APPS_PREFIX);
    let is_callback = path == proxy::APP_CALLBACK_PATH;

    if host == apps_host {
        if is_app_path || is_callback {
            let mut response = next.run(request).await;
            // Pages here are untrusted; make sure none can be framed by or
            // sniffed into anything else on this origin.
            response.headers_mut().insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            return response;
        }
        return StatusCode::NOT_FOUND.into_response();
    }

    if is_callback {
        return StatusCode::NOT_FOUND.into_response();
    }
    if is_app_path {
        let target = request
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or(path);
        return Redirect::temporary(&format!("{apps_url}{target}")).into_response();
    }
    next.run(request).await
}

/// 400 for malformed handles, 404 for missing or cross-user.
async fn resolve_user_app(
    state: &AppState,
    auth: &AuthUser,
    handle: &str,
) -> Result<App, ApiError> {
    let handle = crate::core::Handle::try_new(handle)?;
    let app = state
        .app_service
        .find_by_user_handle(&auth.user_id, &handle)
        .await?
        .ok_or_else(|| {
            ApiError(AppError::NotFound(format!(
                "App '{}' not found",
                handle.as_str()
            )))
        })?;
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
    let resp = state
        .app_service
        .stop(&app.agent_id, &app.id, &app.chat_id)
        .await?;
    Ok(Json(resp))
}

async fn restart_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<AppResponse>, ApiError> {
    let app = resolve_user_app(&state, &auth, &handle).await?;
    let resp = state
        .app_service
        .restart(&app.agent_id, &app.id, &app.chat_id)
        .await?;
    Ok(Json(resp))
}
