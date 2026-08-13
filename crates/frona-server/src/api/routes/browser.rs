use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use super::super::error::ApiError;
use super::super::middleware::auth::{AuthUser, NavigableAuth};
use crate::core::error::AppError;
use crate::core::state::AppState;
use frona_browser::BrowserConnection;

/// Kept short so a bad host/port fails fast instead of hanging the settings UI.
const TEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Presign tokens for the browser debugger are scoped with this owner prefix so
/// a token minted for one credential can't open another.
const DEBUGGER_OWNER: &str = "browser-debugger";
/// The debugger link is minted on demand and opened immediately, so it only
/// needs to survive the click → navigation round-trip.
const DEBUGGER_TOKEN_EXPIRY_SECS: u64 = 300; // 5 minutes

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/browser/debugger/{credential_id}/link",
            get(debugger_link),
        )
        .route("/api/browser/debugger/{credential_id}", get(debugger_proxy))
        .route("/api/browser/test", post(test_browser))
}

#[derive(serde::Deserialize)]
struct TestBrowserRequest {
    ws_url: String,
}

/// Verifies browserless is actually reachable by opening (and immediately
/// tearing down) a real CDP connection, rather than just probing HTTP.
///
/// Builds the error response directly instead of going through `ApiError` —
/// that path collapses `AppError::Browser` down to a generic "Browser
/// service error" for clients, which would defeat the point of a test
/// button that exists to tell the user *why* the connection failed.
async fn test_browser(_auth: AuthUser, Json(req): Json<TestBrowserRequest>) -> Response {
    let ws_url = req.ws_url.trim();
    if ws_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "WebSocket URL is required" })),
        )
            .into_response();
    }

    match BrowserConnection::connect(ws_url, TEST_CONNECT_TIMEOUT, TEST_CONNECT_TIMEOUT).await {
        Ok(conn) => {
            let _ = conn.disconnect().await;
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Failed to connect to browserless: {e}") })),
        )
            .into_response(),
    }
}

/// Mint a short-lived presigned debugger URL. Authenticated normally (called
/// from the web UI via fetch, so the bearer header is present). The returned
/// URL is safe to open in a new tab because its auth rides in the query.
async fn debugger_link(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(credential_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let credential = state
        .vault_service
        .find_credential_by_id(&credential_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(AppError::NotFound("Credential not found".into())))?;
    if credential.user_id != auth.user_id {
        return Err(ApiError::from(AppError::Forbidden("Not your credential".into())));
    }

    let token = state
        .presign_service
        .sign_scoped_token(
            DEBUGGER_OWNER,
            &credential_id,
            &auth.user_id,
            DEBUGGER_TOKEN_EXPIRY_SECS,
        )
        .await?;

    let base = state.config.server.public_base_url();
    let url = format!("{base}/api/browser/debugger/{credential_id}?token={token}");
    Ok(Json(serde_json::json!({ "url": url })))
}

async fn debugger_proxy(
    auth: NavigableAuth,
    State(state): State<AppState>,
    Path(credential_id): Path<String>,
) -> Result<Response, ApiError> {
    // A presign token must be scoped to *this* credential (owner + path).
    auth.require_presign_scope(DEBUGGER_OWNER, &credential_id)?;

    let credential = state
        .vault_service
        .find_credential_by_id(&credential_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(AppError::NotFound("Credential not found".into())))?;

    // Ownership: the authenticated user (or the presign token's subject) must
    // own the credential.
    let user_id = auth.user_id().to_string();
    if credential.user_id != user_id {
        return Err(ApiError::from(AppError::Forbidden("Not your credential".into())));
    }

    // Resolve the handle for the profile path — from the auth for a bearer
    // request, or looked up for a presign request.
    let handle = match &auth {
        NavigableAuth::User { handle, .. } => handle.clone(),
        NavigableAuth::Presigned(_) => {
            state.user_service.handle_of(&user_id).await.map_err(ApiError::from)?
        }
    };

    let browser_config = state.browser_session_manager.config().ok_or_else(|| {
        ApiError::from(AppError::Browser("Browser is not configured".into()))
    })?;
    let browserless_base = browser_config.http_base_url();

    let profile_path = browser_config.profile_path(&handle, &credential.provider);
    let target_url = format!(
        "{}/debugger?--user-data-dir={}",
        browserless_base,
        profile_path.display()
    );

    let client = Client::builder(TokioExecutor::new()).build_http();

    let req = Request::get(&target_url).body(Body::empty()).map_err(|e| {
        ApiError::from(AppError::Browser(format!(
            "Failed to build proxy request: {e}"
        )))
    })?;

    let resp = client.request(req).await.map_err(|e| {
        ApiError::from(AppError::Browser(format!(
            "Failed to proxy to browserless: {e}"
        )))
    })?;

    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.map_err(|e| {
        ApiError::from(AppError::Browser(format!(
            "Failed to read proxy response: {e}"
        )))
    })?;

    Ok(Response::from_parts(parts, Body::from(bytes.to_bytes())))
}
