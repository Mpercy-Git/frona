use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::Stream;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::space::models::{CreateSpaceRequest, SpaceResponse, UpdateSpaceRequest};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/spaces", get(list_spaces).post(create_space))
        .route(
            "/api/spaces/{id}",
            axum::routing::put(update_space).delete(delete_space),
        )
        .route("/api/spaces/{id}/stream", get(space_stream))
}

async fn create_space(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateSpaceRequest>,
) -> Result<Json<SpaceResponse>, ApiError> {
    let response = state.space_service.create(&auth.user_id, req).await?;
    Ok(Json(response))
}

async fn list_spaces(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<SpaceResponse>>, ApiError> {
    let spaces = state.space_service.list(&auth.user_id).await?;
    Ok(Json(spaces))
}

async fn update_space(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSpaceRequest>,
) -> Result<Json<SpaceResponse>, ApiError> {
    let space = state.space_service.update(&auth.user_id, &id, req).await?;
    Ok(Json(space))
}

async fn delete_space(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), ApiError> {
    state.space_service.delete(&auth.user_id, &id).await?;
    Ok(())
}

async fn space_stream(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let _space = state.space_service.get(&auth.user_id, &id).await?;

    let mut raw = state.broadcast_service.subscribe_raw();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Event, Infallible>>();
    let space_id = id.clone();

    tokio::spawn(async move {
        while let Some(event) = raw.recv().await {
            if event.user_id != auth.user_id {
                continue;
            }
            if event.space_id.as_deref() == Some(space_id.as_str())
                && let Some(sse) = crate::chat::broadcast::map_event_to_sse(&event)
                && tx.send(Ok(sse)).is_err()
            {
                break;
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
