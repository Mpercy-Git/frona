use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::contact::models::{ContactResponse, CreateContactRequest, UpdateContactRequest};
use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/contacts", get(list_contacts).post(create_contact))
        .route(
            "/api/contacts/{id}",
            axum::routing::put(update_contact).delete(delete_contact),
        )
}

async fn list_contacts(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ContactResponse>>, ApiError> {
    let contacts = state.contact_service.list(&auth.user_id).await?;
    Ok(Json(contacts))
}

async fn create_contact(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateContactRequest>,
) -> Result<Json<ContactResponse>, ApiError> {
    let response = state.contact_service.create(&auth.user_id, req).await?;
    Ok(Json(response))
}

async fn update_contact(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateContactRequest>,
) -> Result<Json<ContactResponse>, ApiError> {
    let contact = state.contact_service.update(&auth.user_id, &id, req).await?;
    Ok(Json(contact))
}

async fn delete_contact(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), ApiError> {
    state.contact_service.delete(&auth.user_id, &id).await?;
    Ok(())
}
