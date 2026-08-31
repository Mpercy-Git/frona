use crate::core::error::{AppError, AuthErrorCode};
use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Byte-identical 404 for routes that must not leak whether a resource
/// exists. Uses a plain-text body (not `ApiError`'s `{"error": "..."}`) so
/// different failure causes can't be told apart by length.
pub fn anonymous_not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        ApiError(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            // Auth failures answer with a machine-readable `code` alongside the
            // message: an expired session is routine (the client refreshes and
            // retries) and must not be told apart from a real failure by string
            // matching on prose.
            AppError::Auth { message, code } => {
                // Lockout carries a deadline, so it answers with `Retry-After`
                // rather than falling through to the plain (status, body) path.
                if let AuthErrorCode::AccountLocked { retry_after_secs } = code {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        [(header::RETRY_AFTER, retry_after_secs.to_string())],
                        Json(json!({ "error": message, "code": code.as_str() })),
                    )
                        .into_response();
                }
                let status = match code {
                    AuthErrorCode::AccountDeactivated => StatusCode::FORBIDDEN,
                    _ => StatusCode::UNAUTHORIZED,
                };
                return (
                    status,
                    Json(json!({ "error": message, "code": code.as_str() })),
                )
                    .into_response();
            }
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Database(msg) => {
                tracing::error!("Database error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            AppError::Inference(msg) => {
                tracing::error!("Inference error: {msg}");
                (StatusCode::BAD_GATEWAY, "Inference service error".into())
            }
            AppError::Browser(msg) => {
                tracing::error!("Browser error: {msg}");
                (StatusCode::BAD_GATEWAY, "Browser service error".into())
            }
            AppError::Tool(msg) => {
                tracing::error!("Tool error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
            }
            AppError::Decryption(msg) => {
                tracing::error!("Decryption error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            AppError::Http { status, message } => (
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY),
                message.clone(),
            ),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::AuthErrorCode;
    use http_body_util::BodyExt;

    async fn body_json(res: Response) -> serde_json::Value {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The web client branches on `code` to tell an ordinary expiry (refresh
    /// and retry) from a dead session (sign in again), so the field is part of
    /// the contract, not decoration.
    #[tokio::test]
    async fn auth_errors_carry_their_code() {
        let res = ApiError(AppError::Auth {
            message: "Session expired".into(),
            code: AuthErrorCode::TokenExpired,
        })
        .into_response();

        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_json(res).await,
            serde_json::json!({ "error": "Session expired", "code": "token_expired" }),
        );
    }

    #[tokio::test]
    async fn lockout_keeps_retry_after_and_gains_a_code() {
        let res = ApiError(AppError::Auth {
            message: "Too many attempts".into(),
            code: AuthErrorCode::AccountLocked {
                retry_after_secs: 42,
            },
        })
        .into_response();

        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(res.headers()[header::RETRY_AFTER], "42");
        assert_eq!(
            body_json(res).await,
            serde_json::json!({ "error": "Too many attempts", "code": "account_locked" }),
        );
    }
}
