use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use serde::Deserialize;

use super::super::error::ApiError;
use crate::core::error::{AppError, AuthErrorCode};
use crate::core::principal::{Principal, PrincipalKind};
use crate::core::state::AppState;
use crate::storage::PresignClaims;

/// Accepts any principal kind (User, Agent, McpServer, App).
pub struct AuthPrincipal {
    pub user_id: String,
    pub handle: crate::core::Handle,
    pub email: String,
    pub token_id: String,
    pub token_type: String,
    pub principal: Principal,
    pub scopes: Option<Vec<String>>,
    pub extensions: Option<serde_json::Value>,
}

impl AuthPrincipal {
    pub fn agent_id(&self) -> Option<&str> {
        match self.principal.kind {
            PrincipalKind::Agent => Some(&self.principal.id),
            _ => None,
        }
    }
}

impl FromRequestParts<AppState> for AuthPrincipal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_token(parts)?;
        let claims = state
            .token_service
            .validate(&state.keypair_service, token)
            .await?;

        Ok(AuthPrincipal {
            user_id: claims.sub,
            handle: claims.handle,
            email: claims.email,
            token_id: claims.token_id,
            token_type: claims.token_type,
            principal: claims.principal,
            scopes: claims.scopes,
            extensions: claims.extensions,
        })
    }
}

/// Rejects non-User principals.
pub struct AuthUser {
    pub user_id: String,
    pub handle: crate::core::Handle,
    pub email: String,
    pub token_id: String,
    pub token_type: String,
    pub principal: Principal,
    pub scopes: Option<Vec<String>>,
    pub extensions: Option<serde_json::Value>,
}

impl AuthUser {
    pub fn is_pat(&self) -> bool {
        self.token_type == "pat"
    }

    pub fn is_session(&self) -> bool {
        self.token_type == "access"
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes
            .as_ref()
            .is_some_and(|s| s.iter().any(|sc| sc == scope))
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_token(parts)?;
        let claims = state
            .token_service
            .validate(&state.keypair_service, token)
            .await?;

        if claims.principal.kind != PrincipalKind::User {
            return Err(ApiError(AppError::Forbidden(
                "This endpoint requires a user principal".into(),
            )));
        }

        Ok(AuthUser {
            user_id: claims.sub,
            handle: claims.handle,
            email: claims.email,
            token_id: claims.token_id,
            token_type: claims.token_type,
            principal: claims.principal,
            scopes: claims.scopes,
            extensions: claims.extensions,
        })
    }
}

#[derive(Deserialize)]
struct PresignTokenQuery {
    token: Option<String>,
}

/// Auth for endpoints reachable by a plain browser navigation (a new tab or an
/// `<a href>`), which cannot send an `Authorization` header. Accepts EITHER the
/// normal bearer token OR a short-lived presign token in the `?token=` query
/// param (mint one with [`crate::credential::presign::PresignService::sign_scoped_token`]).
///
/// This is the reusable primitive for "hand the user a link they can open":
/// use it instead of `AuthUser` on any GET that a browser navigates to
/// directly. Presign requests are unscoped by construction — the route MUST
/// call [`NavigableAuth::require_presign_scope`] with the resource's expected
/// `owner`/`path` so a token minted for one resource can't open another.
pub enum NavigableAuth {
    User {
        user_id: String,
        handle: crate::core::Handle,
    },
    Presigned(PresignClaims),
}

impl NavigableAuth {
    /// The subject user id, whichever auth path was taken.
    pub fn user_id(&self) -> &str {
        match self {
            NavigableAuth::User { user_id, .. } => user_id,
            NavigableAuth::Presigned(claims) => &claims.sub,
        }
    }

    /// For a presign token, require its claims match the given resource scope.
    /// A bearer-authenticated user passes through (the route does its own
    /// ownership check). Returns `Forbidden` on a scope mismatch.
    pub fn require_presign_scope(&self, owner: &str, path: &str) -> Result<(), ApiError> {
        if let NavigableAuth::Presigned(claims) = self
            && (claims.owner != owner || claims.path != path)
        {
            return Err(ApiError(AppError::Forbidden(
                "Presign token is not valid for this resource".into(),
            )));
        }
        Ok(())
    }
}

impl FromRequestParts<AppState> for NavigableAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Ok(user) = AuthUser::from_request_parts(parts, state).await {
            return Ok(NavigableAuth::User {
                user_id: user.user_id,
                handle: user.handle,
            });
        }

        let query: Query<PresignTokenQuery> =
            Query::try_from_uri(&parts.uri).map_err(|_| {
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
        Ok(NavigableAuth::Presigned(claims))
    }
}

fn extract_token(parts: &Parts) -> Result<&str, ApiError> {
    if let Some(header) = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        return header.strip_prefix("Bearer ").ok_or_else(|| {
            ApiError(AppError::Auth {
                message: "Invalid authorization format".into(),
                code: AuthErrorCode::InvalidCredentials,
            })
        });
    }

    Err(ApiError(AppError::Auth {
        message: "Missing authorization".into(),
        code: AuthErrorCode::InvalidCredentials,
    }))
}
