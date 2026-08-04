use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::SmartIpKeyExtractor;

use crate::api::cookie::{
    extract_refresh_token_from_cookie_header, extract_sso_csrf_from_cookie_header,
    make_clear_refresh_cookie, make_clear_sso_csrf_cookie, make_refresh_cookie,
    make_sso_csrf_cookie,
};
use crate::auth::lockout::{LockStatus, LoginAttemptTracker};
use crate::auth::models::{AuthResponse, ChangePasswordRequest, LoginRequest, RegisterRequest, UpdateProfileRequest, UpdateHandleRequest, UserInfo};
use crate::auth::password_reset::models::{ForgotPasswordRequest, ResetPasswordRequest};
use crate::auth::token::models::CreatePatRequest;
use crate::core::error::{AppError, AuthErrorCode};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    let auth_limit = GovernorConfigBuilder::default()
        .per_second(2)
        .burst_size(5)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .unwrap();

    let refresh_limit = GovernorConfigBuilder::default()
        .per_second(2)
        .burst_size(5)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .unwrap();

    let rate_limited_auth = Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/register", post(register))
        .route("/api/auth/forgot-password", post(forgot_password))
        .route("/api/auth/reset-password", post(reset_password))
        .layer(GovernorLayer::new(auth_limit));

    let rate_limited_refresh = Router::new()
        .route("/api/auth/refresh", post(refresh))
        .layer(GovernorLayer::new(refresh_limit));

    Router::new()
        .merge(rate_limited_auth)
        .merge(rate_limited_refresh)
        .route("/api/auth/me", get(me))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/handle", put(change_handle))
        .route("/api/auth/password", put(change_password))
        .route("/api/auth/profile", put(update_profile))
        .route("/api/auth/tokens", post(create_pat).get(list_pats))
        .route("/api/auth/tokens/{id}", delete(delete_pat))
        .route("/api/auth/config", get(auth_config))
        .route("/api/auth/sso/authorize", get(sso_authorize))
        .route("/api/auth/sso/callback", get(sso_callback))
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, [(axum::http::HeaderName, axum::http::HeaderValue); 1], Json<AuthResponse>), ApiError>
{
    if state.config.sso.disable_local_auth {
        return Err(ApiError(AppError::Validation(
            "SSO registration required".into(),
        )));
    }
    if !state.config.auth.allow_registration {
        return Err(ApiError(AppError::Forbidden(
            "Registration is disabled".into(),
        )));
    }

    let (response, refresh_jwt) = state
        .auth_service
        .register(
            &state.user_service,
            &state.keypair_service,
            &state.token_service,
            &state.policy_service,
            req,
        )
        .await?;
    state
        .agent_service
        .clone_all_builtins_for_user(&response.user.id, &state.storage_service)
        .await?;

    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    let cookie = make_refresh_cookie(
        &refresh_jwt,
        state.token_service.refresh_expiry_secs(),
        secure,
    );

    Ok((
        StatusCode::CREATED,
        [(SET_COOKIE, cookie)],
        Json(response),
    ))
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> Result<([(axum::http::HeaderName, axum::http::HeaderValue); 1], Json<AuthResponse>), ApiError>
{
    if state.config.sso.disable_local_auth {
        return Err(ApiError(AppError::Validation(
            "SSO login required".into(),
        )));
    }

    let identifier = req.identifier.clone();
    // Log the normalized form so the same account reads as one identity across
    // entries, and so log scrapes can't be fragmented by casing.
    let logged_id = LoginAttemptTracker::normalize(&identifier);
    let client_ip = client.ip();

    if let LockStatus::Locked { retry_after } = state.login_tracker.check(&identifier).await {
        // Round up: a sub-second remainder should still ask for 1, never 0.
        let retry_after_secs = retry_after.as_secs().saturating_add(1);
        tracing::warn!(
            identifier = %logged_id,
            client_ip = %client_ip,
            retry_after_secs,
            "Login refused: identifier is locked out"
        );
        crate::core::metrics::record_login_failure("locked_out");
        let minutes = retry_after_secs.div_ceil(60);
        return Err(ApiError(AppError::Auth {
            message: format!(
                "Too many failed attempts. Try again in {minutes} minute{}.",
                if minutes == 1 { "" } else { "s" }
            ),
            code: AuthErrorCode::AccountLocked { retry_after_secs },
        }));
    }

    let result = state
        .auth_service
        .login(
            &state.user_service,
            &state.keypair_service,
            &state.token_service,
            &state.policy_service,
            req,
        )
        .await;

    let (response, refresh_jwt) = match result {
        Ok(v) => {
            state.login_tracker.clear(&identifier).await;
            v
        }
        Err(e) => {
            // Only a genuine credential rejection counts toward lockout. An
            // `Internal` from token minting, or a deactivated account, is not a
            // guess at a password and must not consume the user's budget.
            if matches!(
                &e,
                AppError::Auth {
                    code: AuthErrorCode::InvalidCredentials,
                    ..
                }
            ) {
                let now_locked = state.login_tracker.record_failure(&identifier).await;
                crate::core::metrics::record_login_failure("invalid_credentials");
                tracing::warn!(
                    identifier = %logged_id,
                    client_ip = %client_ip,
                    "Failed login attempt"
                );
                if now_locked {
                    crate::core::metrics::record_account_lockout();
                    tracing::warn!(
                        identifier = %logged_id,
                        client_ip = %client_ip,
                        "Identifier locked out after repeated failed logins"
                    );
                }
            }
            return Err(ApiError(e));
        }
    };

    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    let cookie = make_refresh_cookie(
        &refresh_jwt,
        state.token_service.refresh_expiry_secs(),
        secure,
    );

    Ok(([(SET_COOKIE, cookie)], Json(response)))
}

/// Always answers 202, whether or not the address is registered — the reply
/// must not tell an enumerator which addresses have accounts. Delivery happens
/// on a detached task so the response time doesn't leak it either.
async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    if state.config.sso.disable_local_auth {
        return Err(ApiError(AppError::Validation(
            "SSO login required".into(),
        )));
    }
    // Server-level configuration, not account-level: refusing here reveals
    // nothing about any particular user.
    if state.mail_service.is_none() {
        return Err(ApiError(AppError::Validation(
            "Password reset is not available — this server has no outbound email configured.".into(),
        )));
    }

    let frontend_url = state.config.server.public_frontend_url();
    let expiry_minutes = state.config.auth.password_reset_expiry_minutes;
    let email = req.email;
    let bg = state.clone();
    tokio::spawn(async move {
        let Some(mail) = bg.mail_service.as_ref() else {
            return;
        };
        if let Err(e) = bg
            .password_reset_service
            .send_reset_email(&bg.user_service, mail, &frontend_url, &email, expiry_minutes)
            .await
        {
            tracing::warn!(error = %e, "Password reset email failed");
        }
    });

    Ok(StatusCode::ACCEPTED)
}

async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    if state.config.sso.disable_local_auth {
        return Err(ApiError(AppError::Validation(
            "SSO login required".into(),
        )));
    }

    // Validate the new password before burning the token, so a rejected
    // password doesn't cost the user their one-shot link.
    crate::auth::AuthService::validate_password(&req.new_password)?;

    let user_id = state.password_reset_service.consume(&req.token).await?;
    let user = state
        .auth_service
        .set_password(&state.user_service, &user_id, &req.new_password)
        .await?;

    // Whoever holds the old password — including whoever the user is resetting
    // because of — loses every live session.
    let _ = state
        .token_service
        .repo()
        .delete_by_user_id(&user_id)
        .await;
    state.login_tracker.clear(&user.email).await;
    state.login_tracker.clear(user.handle.as_str()).await;

    tracing::info!(user_id = %user_id, "Password reset completed");
    Ok(StatusCode::NO_CONTENT)
}

async fn me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<UserInfo>, ApiError> {
    let user = state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    let setup_completed = state.get_runtime_config_bool("setup_completed").await;
    let needs_setup = if setup_completed { None } else { Some(true) };

    let info = crate::auth::build_user_info(user, &state.policy_service, needs_setup).await?;
    Ok(Json(info))
}

async fn change_handle(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdateHandleRequest>,
) -> Result<([(axum::http::HeaderName, axum::http::HeaderValue); 1], Json<AuthResponse>), ApiError>
{
    let (response, refresh_jwt) = state
        .auth_service
        .change_handle(
            &state.user_service,
            &state.keypair_service,
            &state.token_service,
            &state.policy_service,
            &state.storage_service,
            &state.config,
            &auth.user_id,
            req,
        )
        .await?;

    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    let cookie = make_refresh_cookie(
        &refresh_jwt,
        state.token_service.refresh_expiry_secs(),
        secure,
    );

    Ok(([(SET_COOKIE, cookie)], Json(response)))
}

async fn change_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<([(axum::http::HeaderName, axum::http::HeaderValue); 1], Json<AuthResponse>), ApiError>
{
    // A leaked PAT must not be upgradable into full account takeover.
    if auth.is_pat() {
        return Err(ApiError(AppError::Forbidden(
            "PATs cannot change the account password".into(),
        )));
    }

    let (response, refresh_jwt) = state
        .auth_service
        .change_password(
            &state.user_service,
            &state.keypair_service,
            &state.token_service,
            &state.policy_service,
            &auth.user_id,
            req,
        )
        .await?;

    // The password just changed, so any lockout from the forgotten one is
    // stale, and any reset link requested beforehand must stop working.
    state.login_tracker.clear(&response.user.email).await;
    state
        .login_tracker
        .clear(response.user.handle.as_str())
        .await;
    state
        .password_reset_service
        .invalidate_for_user(&auth.user_id)
        .await;

    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    let cookie = make_refresh_cookie(
        &refresh_jwt,
        state.token_service.refresh_expiry_secs(),
        secure,
    );

    Ok(([(SET_COOKIE, cookie)], Json(response)))
}

async fn update_profile(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UpdateProfileRequest>,
) -> Result<Json<UserInfo>, ApiError> {
    let user_info = state
        .auth_service
        .update_profile(&state.user_service, &state.policy_service, &auth.user_id, req)
        .await?;
    Ok(Json(user_info))
}

async fn logout(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<([(axum::http::HeaderName, axum::http::HeaderValue); 1], StatusCode), ApiError> {
    if let Some(token) = state
        .token_service
        .repo()
        .find_active_by_id(&auth.token_id)
        .await?
    {
        if let Some(pair_id) = &token.refresh_pair_id {
            state.token_service.revoke_session(pair_id).await?;
        } else {
            state.token_service.repo().delete(&auth.token_id).await?;
        }
    }

    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    Ok((
        [(SET_COOKIE, make_clear_refresh_cookie(secure))],
        StatusCode::NO_CONTENT,
    ))
}

async fn refresh(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<([(axum::http::HeaderName, axum::http::HeaderValue); 1], Json<serde_json::Value>), ApiError>
{
    let refresh_token = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(extract_refresh_token_from_cookie_header)
        .ok_or_else(|| AppError::Auth { message: "Missing refresh token".into(), code: AuthErrorCode::TokenInvalid })?;

    let (access_jwt, new_refresh_jwt, _claims) = state
        .token_service
        .refresh(&state.keypair_service, refresh_token)
        .await?;

    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    let cookie = make_refresh_cookie(
        &new_refresh_jwt,
        state.token_service.refresh_expiry_secs(),
        secure,
    );

    Ok((
        [(SET_COOKIE, cookie)],
        Json(serde_json::json!({ "token": access_jwt })),
    ))
}

async fn create_pat(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePatRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    if auth.is_pat() {
        return Err(ApiError(AppError::Forbidden(
            "PATs cannot create other tokens".into(),
        )));
    }

    let user = state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    let pat = state
        .token_service
        .create_pat(&state.keypair_service, &user, req)
        .await?;

    Ok((StatusCode::CREATED, Json(serde_json::to_value(pat).unwrap())))
}

async fn list_pats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pats = state.token_service.list_pats(&auth.user_id).await?;
    Ok(Json(serde_json::to_value(pats).unwrap()))
}

async fn delete_pat(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .token_service
        .delete_pat(&auth.user_id, &id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Serialize)]
struct SsoStatus {
    enabled: bool,
    disable_local_auth: bool,
}

#[derive(serde::Serialize)]
struct AuthConfigResponse {
    sso: SsoStatus,
    allow_registration: bool,
    /// Drives whether the login page offers a "Forgot password?" link — the
    /// flow is unusable without outbound email.
    password_reset_enabled: bool,
}

async fn auth_config(
    State(state): State<AppState>,
) -> Json<AuthConfigResponse> {
    Json(AuthConfigResponse {
        sso: SsoStatus {
            enabled: state.config.sso.enabled,
            disable_local_auth: state.config.sso.disable_local_auth,
        },
        allow_registration: state.config.auth.allow_registration,
        password_reset_enabled: state.mail_service.is_some()
            && !state.config.sso.disable_local_auth,
    })
}

async fn sso_authorize(
    State(state): State<AppState>,
) -> Result<([(axum::http::HeaderName, axum::http::HeaderValue); 1], axum::response::Redirect), ApiError> {
    let oauth_svc = state
        .oauth_service
        .as_ref()
        .ok_or_else(|| AppError::Validation("SSO is not enabled".into()))?;

    let (auth_url, csrf_secret, _nonce) = oauth_svc.get_authorization_url().await?;
    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    let cookie = make_sso_csrf_cookie(&csrf_secret, secure);
    Ok(([(SET_COOKIE, cookie)], axum::response::Redirect::temporary(&auth_url)))
}

async fn sso_callback(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response
{
    let secure = state.config.server.base_url.as_deref().is_some_and(|u| u.starts_with("https://"));

    match sso_callback_inner(&state, &headers, &params).await {
        Ok(refresh_jwt) => {
            let refresh_cookie = make_refresh_cookie(
                &refresh_jwt,
                state.token_service.refresh_expiry_secs(),
                secure,
            );
            let clear_csrf = make_clear_sso_csrf_cookie(secure);

            axum::response::IntoResponse::into_response((
                axum::response::AppendHeaders([(SET_COOKIE, refresh_cookie), (SET_COOKIE, clear_csrf)]),
                axum::response::Redirect::temporary("/auth/sso/callback"),
            ))
        }
        Err(e) => {
            tracing::warn!(error = %e, "SSO callback failed");
            let clear_csrf = make_clear_sso_csrf_cookie(secure);
            let code = match &e {
                AppError::Auth { code, .. } => code.as_str(),
                _ => AuthErrorCode::ServerError.as_str(),
            };
            let redirect_url = format!("/login?sso_error={code}");

            axum::response::IntoResponse::into_response((
                axum::response::AppendHeaders([(SET_COOKIE, clear_csrf)]),
                axum::response::Redirect::temporary(&redirect_url),
            ))
        }
    }
}

async fn sso_callback_inner(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    params: &std::collections::HashMap<String, String>,
) -> Result<String, AppError> {
    let oauth_svc = state
        .oauth_service
        .as_ref()
        .ok_or_else(|| AppError::Validation("SSO is not enabled".into()))?;

    let callback_state = params
        .get("state")
        .ok_or_else(|| AppError::Validation("Missing state parameter".into()))?;

    let cookie_header = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let csrf_cookie = extract_sso_csrf_from_cookie_header(cookie_header)
        .ok_or_else(|| AppError::Auth { message: "Missing SSO CSRF cookie — please restart the login flow".into(), code: AuthErrorCode::CsrfFailed })?;
    if csrf_cookie != callback_state {
        return Err(AppError::Auth { message: "SSO state mismatch".into(), code: AuthErrorCode::CsrfFailed });
    }

    let code = params
        .get("code")
        .ok_or_else(|| AppError::Validation("Missing authorization code".into()))?;

    let (user, is_new) = oauth_svc
        .handle_callback(
            code,
            callback_state,
            &state.user_service,
            &state.keypair_service,
            &state.token_service,
        )
        .await?;

    if is_new {
        state
            .agent_service
            .clone_all_builtins_for_user(&user.id, &state.storage_service)
            .await?;
    }

    let (_access_jwt, refresh_jwt) = state
        .token_service
        .create_session_pair(&state.keypair_service, &user)
        .await?;

    Ok(refresh_jwt)
}
