use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use crate::chat::models::{ChatResponse, CreateChatRequest, UpdateChatRequest};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/chats", get(list_chats).post(create_chat))
        .route("/api/chats/archived", get(list_archived_chats))
        .route(
            "/api/chats/{id}",
            get(get_chat).put(update_chat).delete(delete_chat),
        )
        .route("/api/chats/{id}/archive", post(archive_chat))
        .route("/api/chats/{id}/unarchive", post(unarchive_chat))
        .route("/api/chats/{id}/delegations", get(list_delegations))
}

/// One delegated sub-task spawned from a chat — enough to show its live status
/// and navigate into the delegate's own chat.
#[derive(serde::Serialize)]
struct DelegationInfo {
    task_id: String,
    agent_id: String,
    agent_name: Option<String>,
    status: crate::agent::task::models::TaskStatus,
    /// The delegate's chat (navigable), if it has started one.
    chat_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Delegation observability: list the delegated sub-tasks this chat spawned,
/// with their agent and live status, so a parent conversation isn't a black
/// box while its delegates run.
async fn list_delegations(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<DelegationInfo>>, ApiError> {
    // Ownership check (returns Forbidden/NotFound if not the user's chat).
    let _ = state.chat_service.get_chat(&auth.user_id, &id).await?;

    let tasks = state.task_service.find_by_source_chat_id(&id).await?;
    let mut out = Vec::new();
    for task in tasks {
        if !matches!(task.kind, crate::agent::task::models::TaskKind::Delegation { .. }) {
            continue;
        }
        let agent_name = state
            .agent_service
            .find_by_id(&task.agent_id)
            .await
            .ok()
            .flatten()
            .map(|a| a.name);
        out.push(DelegationInfo {
            task_id: task.id,
            agent_id: task.agent_id,
            agent_name,
            status: task.status,
            chat_id: task.chat_id,
            created_at: task.created_at,
        });
    }
    Ok(Json(out))
}

async fn create_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateChatRequest>,
) -> Result<Json<ChatResponse>, ApiError> {
    let response = state.chat_service.create_chat(&auth.user_id, req).await?;
    Ok(Json(response))
}

async fn list_chats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ChatResponse>>, ApiError> {
    let chats = state.chat_service.list_chats(&auth.user_id).await?;
    Ok(Json(chats))
}

async fn get_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ChatResponse>, ApiError> {
    let chat = state.chat_service.get_chat(&auth.user_id, &id).await?;
    Ok(Json(chat.into()))
}

async fn update_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChatRequest>,
) -> Result<Json<ChatResponse>, ApiError> {
    let chat = state.chat_service.update_chat(&auth.user_id, &id, req).await?;
    Ok(Json(chat))
}

async fn delete_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), ApiError> {
    state.chat_service.delete_chat(&auth.user_id, &id).await?;
    Ok(())
}

async fn list_archived_chats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ChatResponse>>, ApiError> {
    let chats = state
        .chat_service
        .list_archived_chats(&auth.user_id)
        .await?;
    Ok(Json(chats))
}

async fn archive_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ChatResponse>, ApiError> {
    let chat = state
        .chat_service
        .archive_chat(&auth.user_id, &id)
        .await?;
    Ok(Json(chat))
}

async fn unarchive_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ChatResponse>, ApiError> {
    let chat = state
        .chat_service
        .unarchive_chat(&auth.user_id, &id)
        .await?;
    Ok(Json(chat))
}
