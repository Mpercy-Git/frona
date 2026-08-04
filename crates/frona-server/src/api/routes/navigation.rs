use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::chat::models::ChatResponse;
use crate::space::models::SpaceResponse;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::state::AppState;

#[derive(Debug, Serialize)]
pub struct SpaceWithChats {
    #[serde(flatten)]
    pub space: SpaceResponse,
    pub chats: Vec<ChatResponse>,
}

#[derive(Debug, Serialize)]
pub struct NavigationResponse {
    pub spaces: Vec<SpaceWithChats>,
    pub standalone_chats: Vec<ChatResponse>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/navigation", get(get_navigation))
}

async fn get_navigation(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<NavigationResponse>, ApiError> {
    let spaces = state.space_service.list(&auth.user_id).await?;
    let standalone_chats = state
        .chat_service
        .find_standalone_chats_by_user(&auth.user_id)
        .await?;

    let mut space_with_chats = Vec::new();
    for space in spaces {
        let chats = state
            .chat_service
            .find_user_chats_by_space_id(&space.id)
            .await?;
        space_with_chats.push(SpaceWithChats {
            space,
            chats: chats.into_iter().map(Into::into).collect(),
        });
    }

    // Chats shared with this user have no space membership of their own —
    // surface them alongside the owner's standalone chats.
    let mut standalone_chats: Vec<ChatResponse> =
        standalone_chats.into_iter().map(Into::into).collect();
    standalone_chats.extend(state.chat_service.shared_chat_responses(&auth.user_id).await?);

    Ok(Json(NavigationResponse {
        spaces: space_with_chats,
        standalone_chats,
    }))
}
