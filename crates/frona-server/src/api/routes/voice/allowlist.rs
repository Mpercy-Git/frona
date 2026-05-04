use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::error::ApiError;
use crate::api::middleware::auth::AuthUser;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/voice/allowlist",
            get(list_allowlist).post(add_to_allowlist),
        )
        .route(
            "/api/voice/allowlist/{phone}",
            axum::routing::delete(remove_from_allowlist),
        )
}

#[derive(Deserialize)]
struct AllowlistRequest {
    phone: String,
}

/// `GET /api/voice/allowlist` — return the authenticated user's allow-list.
async fn list_allowlist(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, ApiError> {
    let numbers = state.get_allowlist(&auth.user_id).await;
    Ok(Json(numbers))
}

/// `POST /api/voice/allowlist` — add a phone number to the user's allow-list.
async fn add_to_allowlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<AllowlistRequest>,
) -> Result<Json<Vec<String>>, ApiError> {
    state.add_to_allowlist(&auth.user_id, &req.phone).await?;
    let numbers = state.get_allowlist(&auth.user_id).await;
    Ok(Json(numbers))
}

/// `DELETE /api/voice/allowlist/{phone}` — remove a number from the user's
/// allow-list.  The `phone` path segment should be URL-encoded (e.g. `%2B1…`).
async fn remove_from_allowlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(phone): Path<String>,
) -> Result<Json<Vec<String>>, ApiError> {
    state.remove_from_allowlist(&auth.user_id, &phone).await?;
    let numbers = state.get_allowlist(&auth.user_id).await;
    Ok(Json(numbers))
}

#[cfg(test)]
mod tests {
    use crate::tool::voice::normalize_phone;

    #[test]
    fn normalize_strips_formatting() {
        assert_eq!(normalize_phone("+1 (555) 555-1234"), "+15555551234");
        assert_eq!(normalize_phone("+44 20 7946 0958"), "+442079460958");
    }

    #[test]
    fn normalize_preserves_plain_e164() {
        assert_eq!(normalize_phone("+15555551234"), "+15555551234");
    }

    #[test]
    fn normalize_no_plus_prefix() {
        assert_eq!(normalize_phone("15555551234"), "15555551234");
    }

    #[test]
    fn normalize_empty() {
        assert_eq!(normalize_phone(""), "");
    }
}
