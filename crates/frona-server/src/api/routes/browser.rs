use axum::body::Body;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::Request;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::error::{AppError, AuthErrorCode};
use crate::core::state::AppState;

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
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Auth for the debugger proxy: a plain browser navigation (new tab) can't send
/// an `Authorization` header, so accept either the normal bearer token OR a
/// short-lived presign token in the `?token=` query param (minted by
/// `debugger_link`). Mirrors the presign fallback used for file downloads.
enum DebuggerAuth {
    User { user_id: String, handle: crate::core::Handle },
    Presigned { user_id: String, credential_id: String },
}

impl FromRequestParts<AppState> for DebuggerAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Ok(auth) = AuthUser::from_request_parts(parts, state).await {
            return Ok(DebuggerAuth::User {
                user_id: auth.user_id,
                handle: auth.handle,
            });
        }

        let query: Query<TokenQuery> = Query::try_from_uri(&parts.uri).map_err(|_| {
            ApiError(AppError::Auth {
                message: "Missing authorization".into(),
                code: AuthErrorCode::InvalidCredentials,
            })
        })?;
        let token = query.token.as_deref().ok_or_else(|| {
            ApiError(AppError::Auth {
                message: "Missing authorization".into(),
                code: AuthErrorCode::InvalidCredentials,
            })
        })?;

        let claims = state.presign_service.verify(token).await?;
        if claims.owner != DEBUGGER_OWNER {
            return Err(ApiError(AppError::Auth {
                message: "Token not valid for browser debugger".into(),
                code: AuthErrorCode::TokenInvalid,
            }));
        }
        Ok(DebuggerAuth::Presigned {
            user_id: claims.sub,
            credential_id: claims.path,
        })
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
    debug_auth: DebuggerAuth,
    State(state): State<AppState>,
    Path(credential_id): Path<String>,
) -> Result<Response, ApiError> {
    let credential = state
        .vault_service
        .find_credential_by_id(&credential_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(AppError::NotFound("Credential not found".into())))?;

    // Ownership: the authenticated user (or the presign token's subject) must
    // own the credential, and a presign token must be for *this* credential.
    let user_id = match &debug_auth {
        DebuggerAuth::User { user_id, .. } => user_id.clone(),
        DebuggerAuth::Presigned { user_id, credential_id: tok_cred } => {
            if tok_cred != &credential_id {
                return Err(ApiError::from(AppError::Forbidden(
                    "Token does not match this credential".into(),
                )));
            }
            user_id.clone()
        }
    };
    if credential.user_id != user_id {
        return Err(ApiError::from(AppError::Forbidden("Not your credential".into())));
    }

    // Resolve the handle for the profile path — from the auth for a user
    // request, or looked up for a presign request.
    let handle = match &debug_auth {
        DebuggerAuth::User { handle, .. } => handle.clone(),
        DebuggerAuth::Presigned { .. } => {
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
