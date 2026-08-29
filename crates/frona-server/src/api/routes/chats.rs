use crate::chat::models::{ChatResponse, CreateChatRequest, UpdateChatRequest};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

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
        .route(
            "/api/chats/{id}/shares",
            get(list_chat_shares).post(share_chat),
        )
        .route(
            "/api/chats/{id}/shares/{recipient_id}",
            axum::routing::delete(unshare_chat),
        )
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
    // Owner or shared-recipient may view; `get_accessible` returns Forbidden
    // otherwise. Editing endpoints stay owner-only (via `get_chat`).
    let (chat, is_owner) = state.chat_service.get_accessible(&auth.user_id, &id).await?;
    let response: ChatResponse = if is_owner {
        chat.into()
    } else {
        let shared_by = state.user_service.handle_of(&chat.user_id).await.ok();
        let mut response: ChatResponse = chat.into();
        response.is_shared = true;
        response.shared_by = shared_by.map(|h| h.to_string());
        response
    };
    Ok(Json(response))
}

async fn update_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChatRequest>,
) -> Result<Json<ChatResponse>, ApiError> {
    let chat = state
        .chat_service
        .update_chat(&auth.user_id, &id, req)
        .await?;
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
    let chat = state.chat_service.archive_chat(&auth.user_id, &id).await?;
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

// ---------------------------------------------------------------------------
// Chat sharing (read-only, per recipient)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ShareChatRequest {
    /// Recipient username/handle or email.
    recipient: String,
}

#[derive(Serialize)]
struct ChatShareResponse {
    recipient_id: String,
    recipient_handle: String,
    recipient_name: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn build_chat_share_responses(
    state: &AppState,
    shares: Vec<crate::chat::share::models::ChatShare>,
) -> Vec<ChatShareResponse> {
    let mut out = Vec::with_capacity(shares.len());
    for s in shares {
        // Resolve recipient display info; fall back to the id if the user row
        // has since vanished.
        let (handle, name) = match state.user_service.find_by_id(&s.recipient_id).await {
            Ok(Some(u)) => (u.handle.to_string(), u.name),
            _ => (s.recipient_id.clone(), String::new()),
        };
        out.push(ChatShareResponse {
            recipient_id: s.recipient_id,
            recipient_handle: handle,
            recipient_name: name,
            created_at: s.created_at,
        });
    }
    out
}

/// `GET /api/chats/{id}/shares` — who this chat is shared with (owner only).
async fn list_chat_shares(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ChatShareResponse>>, ApiError> {
    // Owner-only: `get_chat` returns Forbidden for non-owners.
    let _ = state.chat_service.get_chat(&auth.user_id, &id).await?;
    let shares = state.chat_share_service.list_for_chat(&id).await?;
    Ok(Json(build_chat_share_responses(&state, shares).await))
}

/// `POST /api/chats/{id}/shares` — grant a user read-only access (owner only).
async fn share_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ShareChatRequest>,
) -> Result<Json<Vec<ChatShareResponse>>, ApiError> {
    let _ = state.chat_service.get_chat(&auth.user_id, &id).await?;
    state
        .chat_share_service
        .share(&auth.user_id, &id, &req.recipient)
        .await?;
    let shares = state.chat_share_service.list_for_chat(&id).await?;
    Ok(Json(build_chat_share_responses(&state, shares).await))
}

/// `DELETE /api/chats/{id}/shares/{recipient_id}` — revoke access (owner only).
async fn unshare_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, recipient_id)): Path<(String, String)>,
) -> Result<Json<Vec<ChatShareResponse>>, ApiError> {
    let _ = state.chat_service.get_chat(&auth.user_id, &id).await?;
    state.chat_share_service.unshare(&id, &recipient_id).await?;
    let shares = state.chat_share_service.list_for_chat(&id).await?;
    Ok(Json(build_chat_share_responses(&state, shares).await))
}
