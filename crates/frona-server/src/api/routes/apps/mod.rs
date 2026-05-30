mod models;
mod proxy;

use axum::extract::{Path, State};
use axum::routing::{any, get, post};
use axum::{Json, Router};

use crate::app::models::{App, AppResponse};
use crate::core::error::AppError;
use crate::core::state::AppState;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

use models::ServiceActionRequest;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps", get(list_apps))
        .route("/api/apps/{handle}", get(get_app).delete(delete_app))
        .route("/api/apps/{handle}/stop", post(stop_app))
        .route("/api/apps/{handle}/restart", post(restart_app))
        .route("/api/apps/approve", post(approve_service))
        .route("/api/apps/deny", post(deny_service))
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

async fn approve_service(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ServiceActionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let chat = state
        .chat_service
        .get_chat(&auth.user_id, &req.chat_id)
        .await
        .map_err(ApiError::from)?;

    let pending_te = state
        .chat_service
        .find_pending_tool_call(&req.chat_id)
        .await
        .map_err(ApiError::from)?;

    let pending_te = pending_te.ok_or_else(|| {
        ApiError::from(crate::core::error::AppError::NotFound(
            "No pending service approval found".into(),
        ))
    })?;

    let manifest_value = match &pending_te.tool_data {
        Some(crate::inference::tool_call::MessageTool::ServiceApproval {
            manifest,
            ..
        }) => manifest.clone(),
        _ => {
            return Err(ApiError::from(crate::core::error::AppError::NotFound(
                "No pending service approval found".into(),
            )));
        }
    };

    let manifest: crate::app::models::AppManifest =
        serde_json::from_value(manifest_value).map_err(|e| {
            ApiError::from(crate::core::error::AppError::Validation(format!(
                "Invalid manifest: {e}"
            )))
        })?;

    let te_id = pending_te.id.clone();
    let te_message_id = pending_te.message_id.clone();

    let user_id = auth.user_id.clone();
    let chat_id = req.chat_id.clone();
    let agent_id = chat.agent_id.clone();
    let manifest_name = manifest.name.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        let base_url = state_clone.config.server.public_base_url();

        let (result_text, level, title, body, app_handle) = match state_clone
            .app_service
            .deploy_and_await(&agent_id, &user_id, &chat_id, &manifest, Vec::new())
            .await
        {
            Ok(app) => {
                let app_url = format!("{base_url}{}", app.url.as_deref().unwrap_or(""));
                let handle = app.handle.clone();
                (
                    crate::tool::manage_service::format_app_result("deployed successfully", &app),
                    crate::notification::models::NotificationLevel::Success,
                    format!("App '{}' deployed", manifest_name),
                    app_url,
                    handle,
                )
            }
            Err(e) => (
                format!("Deploy failed: {e}"),
                crate::notification::models::NotificationLevel::Error,
                format!("Deploy failed: '{}'", manifest_name),
                e.to_string(),
                String::new(),
            ),
        };

        if let Ok(result) = state_clone
            .chat_service
            .resolve_tool_call(&te_id, Some(result_text))
            .await
        {
            state_clone.broadcast_service.send(crate::chat::broadcast::BroadcastEvent {
                user_id: user_id.clone(),
                chat_id: Some(chat_id.clone()),
                space_id: None,
                kind: crate::chat::broadcast::BroadcastEventKind::ToolResolved {
                    message: result.into_message(),
                },
            });
        }

        if let Ok(notification) = state_clone
            .notification_service
            .create(
                &user_id,
                crate::notification::models::NotificationData::App {
                    app_handle,
                    action: "deploy".to_string(),
                },
                level,
                title,
                body,
            )
            .await
        {
            state_clone.broadcast_service.send_notification(&user_id, notification);
        }

        let still_pending = state_clone.chat_service
            .has_pending_tools_for_message(&te_message_id).await.unwrap_or(false);
        if !still_pending {
            crate::agent::task::executor::resume_or_notify(&state_clone, &user_id, &chat_id, &te_message_id).await;
        }
    });

    Ok(Json(serde_json::json!({ "approved": true })))
}

async fn deny_service(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ServiceActionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .chat_service
        .get_chat(&auth.user_id, &req.chat_id)
        .await
        .map_err(ApiError::from)?;

    let pending_te = state
        .chat_service
        .find_pending_tool_call(&req.chat_id)
        .await
        .map_err(ApiError::from)?;

    if let Some(te) = pending_te {
        let message_id = te.message_id.clone();
        let denied = state
            .chat_service
            .deny_tool_call(
                &te.id,
                Some("User denied the service deployment.".to_string()),
            )
            .await
            .map_err(ApiError::from)?
            .into_message();

        state.broadcast_service.send(crate::chat::broadcast::BroadcastEvent {
            user_id: auth.user_id.clone(),
            chat_id: Some(req.chat_id.clone()),
            space_id: None,
            kind: crate::chat::broadcast::BroadcastEventKind::ToolResolved { message: denied },
        });

        let still_pending = state.chat_service
            .has_pending_tools_for_message(&message_id).await.unwrap_or(false);
        if !still_pending {
            let user_id = auth.user_id.clone();
            let chat_id = req.chat_id.clone();
            let state_clone = state.clone();
            tokio::spawn(async move {
                crate::agent::task::executor::resume_or_notify(&state_clone, &user_id, &chat_id, &message_id).await;
            });
        }
    }

    Ok(Json(serde_json::json!({ "denied": true })))
}
