use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use openidconnect::core::{CoreClient, CoreProviderMetadata, CoreUserInfoClaims};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, OAuth2TokenResponse,
    RedirectUrl, Scope, SubjectIdentifier,
};
use tokio::sync::Mutex;

use super::models::OAuthIdentity;
use super::repository::OAuthRepository;
use crate::auth::token::service::TokenService;
use crate::auth::{AuthService, User, UserService};
use crate::core::config::Config;
use crate::core::error::{AppError, AuthErrorCode};
use crate::credential::keypair::service::KeyPairService;

#[derive(Clone)]
pub struct OAuthService {
    authority: String,
    client_id: String,
    client_secret: String,
    scopes: Vec<String>,
    allow_unknown_email_verification: bool,
    signups_match_email: bool,
    // Maps CSRF state → (nonce_secret, Nonce, expiry). Entries are pruned on
    // insert to prevent unbounded growth from abandoned authorize flows.
    pending_states: Arc<Mutex<HashMap<String, (String, Nonce, chrono::DateTime<Utc>)>>>,
    repo: Arc<dyn OAuthRepository>,
    redirect_uri: String,
    http: openidconnect::reqwest::Client,
}

impl OAuthService {
    pub fn new(config: &Config, repo: Arc<dyn OAuthRepository>) -> Result<Self, AppError> {
        let authority = config.sso.authority.clone().ok_or_else(|| {
            AppError::Validation("FRONA_SSO_AUTHORITY is required when SSO is enabled".into())
        })?;
        let client_id = config.sso.client_id.clone().ok_or_else(|| {
            AppError::Validation("FRONA_SSO_CLIENT_ID is required when SSO is enabled".into())
        })?;
        let client_secret = config.sso.client_secret.clone().ok_or_else(|| {
            AppError::Validation("FRONA_SSO_CLIENT_SECRET is required when SSO is enabled".into())
        })?;

        let scopes: Vec<String> = config
            .sso
            .scopes
            .split_whitespace()
            .map(String::from)
            .collect();
        let base = config.server.public_base_url();
        if base.is_empty() {
            return Err(AppError::Validation(
                "SSO requires server.base_url or server.backend_url to be set".into(),
            ));
        }
        let redirect_uri = format!("{base}/api/auth/sso/callback");

        Ok(Self {
            authority,
            client_id,
            client_secret,
            scopes,
            allow_unknown_email_verification: config.sso.allow_unknown_email_verification,
            signups_match_email: config.sso.signups_match_email,
            pending_states: Arc::new(Mutex::new(HashMap::<String, (String, Nonce, chrono::DateTime<Utc>)>::new())),
            repo,
            redirect_uri,
            http: openidconnect::reqwest::Client::new(),
        })
    }

    fn issuer_url(&self) -> Result<IssuerUrl, AppError> {
        IssuerUrl::new(self.authority.clone())
            .map_err(|e| AppError::Internal(format!("Invalid SSO authority URL: {e}")))
    }

    fn redirect_url(&self) -> Result<RedirectUrl, AppError> {
        RedirectUrl::new(self.redirect_uri.clone())
            .map_err(|e| AppError::Internal(format!("Invalid redirect URI: {e}")))
    }

    pub async fn get_authorization_url(&self) -> Result<(String, String, String), AppError> {
        let http_client = self.http.clone();
        let issuer_url = self.issuer_url()?;

        let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
            .await
            .map_err(|e| AppError::Internal(format!("OIDC discovery failed: {e}")))?;

        let client = openidconnect::core::CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(self.client_id.clone()),
            Some(ClientSecret::new(self.client_secret.clone())),
        )
        .set_redirect_uri(self.redirect_url()?);

        let mut auth_request = client.authorize_url(
            openidconnect::AuthenticationFlow::<openidconnect::core::CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        );

        for scope in &self.scopes {
            auth_request = auth_request.add_scope(Scope::new(scope.clone()));
        }

        let (auth_url, csrf_state, nonce) = auth_request.url();

        let csrf_secret = csrf_state.secret().clone();
        let nonce_secret = nonce.secret().clone();

        const STATE_TTL_MINUTES: i64 = 10;
        let expiry = Utc::now() + chrono::Duration::minutes(STATE_TTL_MINUTES);

        let mut states = self.pending_states.lock().await;
        // Prune expired entries so the map doesn't grow without bound.
        let now = Utc::now();
        states.retain(|_, (_, _, exp)| *exp > now);
        states.insert(csrf_secret.clone(), (nonce_secret.clone(), nonce, expiry));
        drop(states);

        Ok((auth_url.to_string(), csrf_secret, nonce_secret))
    }

    pub async fn handle_callback(
        &self,
        code: &str,
        state: &str,
        user_service: &UserService,
        _keypair_svc: &KeyPairService,
        _token_svc: &TokenService,
    ) -> Result<(User, bool), AppError> {
        let (_nonce_secret, nonce, _expiry) = self
            .pending_states
            .lock()
            .await
            .remove(state)
            .ok_or_else(|| AppError::Auth { message: "Invalid or expired SSO state".into(), code: AuthErrorCode::CsrfFailed })?;

        let http_client = self.http.clone();
        let issuer_url = self.issuer_url()?;

        let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
            .await
            .map_err(|e| AppError::Internal(format!("OIDC discovery failed: {e}")))?;

        let client = CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(self.client_id.clone()),
            Some(ClientSecret::new(self.client_secret.clone())),
        )
        .set_redirect_uri(self.redirect_url()?);

        let token_response = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| AppError::Internal(format!("Token endpoint configuration error: {e}")))?
            .request_async(&http_client)
            .await
            .map_err(|e| AppError::Internal(format!("Token exchange failed: {e}")))?;

        let id_token = token_response
            .extra_fields()
            .id_token()
            .ok_or_else(|| AppError::Auth {
                message: "No ID token in response".into(),
                code: AuthErrorCode::TokenFailed,
            })?;

        let id_token_verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&id_token_verifier, &nonce)
            .map_err(|e| AppError::Auth {
                message: format!("ID token validation failed: {e}"),
                code: AuthErrorCode::TokenInvalid,
            })?;

        let external_sub = claims.subject().to_string();
        let mut external_email = claims
            .email()
            .map(|e| AuthService::normalize_email(e.as_str()));
        let mut external_name = pick_name(
            claims.name().and_then(|n| n.get(None)).map(|n| n.as_str()),
            claims
                .given_name()
                .and_then(|n| n.get(None))
                .map(|n| n.as_str()),
            claims
                .family_name()
                .and_then(|n| n.get(None))
                .map(|n| n.as_str()),
            claims.preferred_username().map(|n| n.as_str()),
        );

        if !self.allow_unknown_email_verification
            && let Some(verified) = claims.email_verified()
            && !verified
        {
            return Err(AppError::Auth {
                message: "Email not verified by SSO provider".into(),
                code: AuthErrorCode::EmailNotVerified,
            });
        }

        if let Some(identity) = self.repo.find_identity_by_sub(&external_sub).await? {
            match user_service.find_by_id(&identity.user_id).await? {
                Some(user) => {
                    if user.deactivated_at.is_some() {
                        return Err(AppError::Auth {
                            message: "Account deactivated".into(),
                            code: AuthErrorCode::AccountDeactivated,
                        });
                    }
                    return Ok((user, false));
                }
                None => {
                    tracing::warn!(
                        identity_id = %identity.id,
                        user_id = %identity.user_id,
                        "Dropping orphaned SSO identity whose user no longer exists"
                    );
                    self.repo.delete(&identity.id).await?;
                }
            }
        }

        let mut matched_user: Option<User> = None;
        if self.signups_match_email
            && let Some(ref email) = external_email
        {
            matched_user = user_service.find_by_email(email).await?;
        }

        // Fall back to the userinfo endpoint when the ID token alone didn't
        // yield a mergeable email. Some IdPs (e.g. configurations that emit
        // bare-sub ID tokens) only return email/name from userinfo.
        if self.signups_match_email && matched_user.is_none() {
            match fetch_userinfo(&client, &http_client, &token_response, &external_sub).await {
                Ok(Some(info)) => {
                    let info_email = info
                        .email()
                        .map(|e| AuthService::normalize_email(e.as_str()));
                    let info_name = pick_name(
                        info.name().and_then(|n| n.get(None)).map(|n| n.as_str()),
                        info.given_name()
                            .and_then(|n| n.get(None))
                            .map(|n| n.as_str()),
                        info.family_name()
                            .and_then(|n| n.get(None))
                            .map(|n| n.as_str()),
                        info.preferred_username().map(|n| n.as_str()),
                    );
                    if external_name.is_none() {
                        external_name = info_name;
                    }
                    if let Some(email) = info_email {
                        if external_email.as_deref() != Some(email.as_str()) {
                            matched_user = user_service.find_by_email(&email).await?;
                        }
                        if external_email.is_none() {
                            external_email = Some(email);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "UserInfo fetch failed, falling back to ID token claims only");
                }
            }
        }

        if let Some(existing_user) = matched_user {
            if existing_user.deactivated_at.is_some() {
                return Err(AppError::Auth {
                    message: "Account deactivated".into(),
                    code: AuthErrorCode::AccountDeactivated,
                });
            }
            let now = Utc::now();
            let identity = OAuthIdentity {
                id: crate::core::repository::new_id(),
                user_id: existing_user.id.clone(),
                external_sub,
                external_email: external_email.clone(),
                external_name,
                created_at: now,
                updated_at: now,
            };
            self.repo.create(&identity).await?;
            return Ok((existing_user, false));
        }

        let now = Utc::now();
        let base_handle = if let Some(ref email) = external_email {
            AuthService::derive_handle_from_email(email)
        } else {
            format!("sso-{external_sub}")
        };
        let handle = AuthService::generate_unique_handle(user_service, &base_handle).await?;

        let new_user = User {
            id: crate::core::repository::new_id(),
            handle,
            email: external_email
                .clone()
                .unwrap_or_else(|| format!("sso-{external_sub}@unknown")),
            name: external_name
                .clone()
                .or_else(|| {
                    external_email
                        .as_deref()
                        .and_then(|e| e.split('@').next())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "SSO User".to_string()),
            password_hash: String::new(),
            timezone: None,
            phone: None,
            groups: Vec::new(),
            deactivated_at: None,
            created_at: now,
            updated_at: now,
        };
        let user = user_service.create(&new_user).await?;
        user_service.ensure_admin_invariant().await?;

        let identity = OAuthIdentity {
            id: crate::core::repository::new_id(),
            user_id: user.id.clone(),
            external_sub,
            external_email,
            external_name,
            created_at: now,
            updated_at: now,
        };
        self.repo.create(&identity).await?;

        Ok((user, true))
    }
}

fn pick_name(
    name: Option<&str>,
    given_name: Option<&str>,
    family_name: Option<&str>,
    preferred_username: Option<&str>,
) -> Option<String> {
    fn trimmed(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    if let Some(s) = trimmed(name) {
        return Some(s.to_string());
    }
    let given = trimmed(given_name);
    let family = trimmed(family_name);
    if given.is_some() || family.is_some() {
        let mut out = String::new();
        if let Some(g) = given {
            out.push_str(g);
        }
        if let Some(f) = family {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(f);
        }
        return Some(out);
    }
    trimmed(preferred_username).map(str::to_string)
}

async fn fetch_userinfo(
    client: &CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    >,
    http_client: &openidconnect::reqwest::Client,
    token_response: &openidconnect::core::CoreTokenResponse,
    expected_sub: &str,
) -> Result<Option<CoreUserInfoClaims>, String> {
    let request = match client.user_info(
        token_response.access_token().to_owned(),
        Some(SubjectIdentifier::new(expected_sub.to_string())),
    ) {
        Ok(req) => req,
        Err(_) => return Ok(None),
    };
    request
        .request_async(http_client)
        .await
        .map(Some)
        .map_err(|e| e.to_string())
}
