mod live;

use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use super::super::middleware::auth::AuthUser;
use crate::core::config::BrowserConfig;
use crate::core::state::AppState;
use crate::tool::browser::session::BrowserSessionManager;

/// Kept short so a bad host/port fails fast instead of hanging the settings UI.
const TEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/browser/live/link", get(live::live_link))
        .route("/api/browser/live/ws", get(live::live_socket))
        .route("/api/browser/test", post(test_browser))
}

#[derive(serde::Deserialize)]
struct TestBrowserRequest {
    ws_url: String,
    #[serde(default)]
    profiles_path: Option<String>,
}

/// Verifies browserless is actually reachable by opening (and immediately
/// tearing down) a real CDP connection, rather than just probing HTTP.
///
/// Connects through [`BrowserSessionManager::test_connection`] so this
/// exercises the same `--user-data-dir` / `timeout` launch args every real
/// browser session uses — connecting to the bare WS endpoint alone can
/// succeed even when browserless can't actually launch a session with a
/// persistent profile (e.g. an unwritable profiles volume), which would let
/// the settings-page test pass while every real browser-tool call keeps
/// failing.
///
/// Builds the error response directly instead of going through `ApiError` —
/// that path collapses `AppError::Browser` down to a generic "Browser
/// service error" for clients, which would defeat the point of a test
/// button that exists to tell the user *why* the connection failed.
async fn test_browser(auth: AuthUser, Json(req): Json<TestBrowserRequest>) -> Response {
    let ws_url = req.ws_url.trim();
    if ws_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "WebSocket URL is required" })),
        )
            .into_response();
    }

    let config = BrowserConfig {
        ws_url: ws_url.to_string(),
        profiles_path: req
            .profiles_path
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| BrowserConfig::default().profiles_path),
        ..BrowserConfig::default()
    };

    match BrowserSessionManager::test_connection(&auth.handle, &config, TEST_CONNECT_TIMEOUT).await
    {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Failed to connect to browserless: {e}") })),
        )
            .into_response(),
    }
}
