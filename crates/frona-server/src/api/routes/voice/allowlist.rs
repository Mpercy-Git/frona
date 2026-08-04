use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::api::middleware::auth::AuthUser;
use crate::core::state::AllowlistEntry;
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
        .route(
            "/api/voice/inbound-settings",
            get(get_inbound_settings).put(set_inbound_settings),
        )
}

#[derive(Deserialize)]
struct AllowlistRequest {
    phone: String,
    /// Optional display name for the caller (e.g. "Mum", "Dr. Smith").
    /// When set, the agent will be told this name when the caller dials in.
    #[serde(default)]
    name: Option<String>,
}

/// `GET /api/voice/allowlist` — return the authenticated user's allow-list.
async fn list_allowlist(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AllowlistEntry>>, ApiError> {
    let entries = state.get_allowlist(&auth.user_id).await;
    Ok(Json(entries))
}

/// `POST /api/voice/allowlist` — add a phone number (with optional name) to
/// the user's allow-list.
async fn add_to_allowlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<AllowlistRequest>,
) -> Result<Json<Vec<AllowlistEntry>>, ApiError> {
    state
        .add_to_allowlist(&auth.user_id, &req.phone, req.name.as_deref())
        .await?;
    let entries = state.get_allowlist(&auth.user_id).await;
    Ok(Json(entries))
}

/// `DELETE /api/voice/allowlist/{phone}` — remove a number from the user's
/// allow-list.  The `phone` path segment should be URL-encoded (e.g. `%2B1…`).
async fn remove_from_allowlist(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(phone): Path<String>,
) -> Result<Json<Vec<AllowlistEntry>>, ApiError> {
    state.remove_from_allowlist(&auth.user_id, &phone).await?;
    let entries = state.get_allowlist(&auth.user_id).await;
    Ok(Json(entries))
}

// ---------------------------------------------------------------------------
// Per-user inbound answering settings (agent + welcome greeting)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct InboundSettingsResponse {
    /// The user's chosen inbound agent (ID, handle, or name), or `null` when
    /// they haven't set one (calls fall back to their `receptionist`).
    agent: Option<String>,
    /// The user's inbound welcome greeting, or `null` when unset (the
    /// server-level default greeting applies).
    greeting: Option<String>,
}

#[derive(Deserialize)]
struct InboundSettingsRequest {
    /// Agent ID, handle, or name. Blank clears the override.
    #[serde(default)]
    agent: String,
    /// Welcome greeting spoken when a call connects. Blank clears it.
    #[serde(default)]
    greeting: String,
}

async fn read_inbound_settings(state: &AppState, user_id: &str) -> InboundSettingsResponse {
    InboundSettingsResponse {
        agent: state.get_inbound_agent(user_id).await,
        greeting: state.get_inbound_greeting(user_id).await,
    }
}

/// `GET /api/voice/inbound-settings` — return the authenticated user's inbound
/// answering agent and welcome greeting (each `null` if unset).
async fn get_inbound_settings(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<InboundSettingsResponse>, ApiError> {
    Ok(Json(read_inbound_settings(&state, &auth.user_id).await))
}

/// `PUT /api/voice/inbound-settings` — set (or clear) the authenticated user's
/// inbound answering agent and welcome greeting. Blank values clear each.
async fn set_inbound_settings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<InboundSettingsRequest>,
) -> Result<Json<InboundSettingsResponse>, ApiError> {
    state.set_inbound_agent(&auth.user_id, &req.agent).await?;
    state.set_inbound_greeting(&auth.user_id, &req.greeting).await?;
    Ok(Json(read_inbound_settings(&state, &auth.user_id).await))
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

    #[test]
    fn normalize_00_prefix_uk() {
        // UK international dialling prefix "00" should produce same result as "+".
        assert_eq!(normalize_phone("00442079460958"), "+442079460958");
        assert_eq!(normalize_phone("0044 20 7946 0958"), "+442079460958");
    }

    #[test]
    fn normalize_00_prefix_matches_plus_prefix() {
        // A number stored as +44... must match an incoming 0044... and vice-versa.
        assert_eq!(
            normalize_phone("00442079460958"),
            normalize_phone("+44 20 7946 0958")
        );
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_phone("  +44 20 7946 0958  "), "+442079460958");
    }
}
