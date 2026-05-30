use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use openidconnect::core::{CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    RedirectUrl, Scope,
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
    pending_states: Arc<Mutex<HashMap<String, (String, Nonce)>>>,
    repo: Arc<dyn OAuthRepository>,
    redirect_uri: String,
    http: reqwest::Client,
}

impl OAuthService {
    pub fn new(
        config: &Config,
        repo: Arc<dyn OAuthRepository>,
        http: reqwest::Client,
    ) -> Result<Self, AppError> {
        let authority = config
            .sso
            .authority
            .clone()
            .ok_or_else(|| {
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
            pending_states: Arc::new(Mutex::new(HashMap::new())),
            repo,
            redirect_uri,
            http,
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

    pub async fn get_authorization_url(
        &self,
    ) -> Result<(String, String, String), AppError> {
        let http_client = self.http.clone();
        let issuer_url = self.issuer_url()?;

        let provider_metadata =
            CoreProviderMetadata::discover_async(issuer_url, &http_client)
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

        self.pending_states
            .lock()
            .await
            .insert(csrf_secret.clone(), (nonce_secret.clone(), nonce));

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
        let (_nonce_secret, nonce) = self
            .pending_states
            .lock()
            .await
            .remove(state)
            .ok_or_else(|| AppError::Auth { message: "Invalid or expired SSO state".into(), code: AuthErrorCode::CsrfFailed })?;

        let http_client = self.http.clone();
        let issuer_url = self.issuer_url()?;

        let provider_metadata =
            CoreProviderMetadata::discover_async(issuer_url, &http_client)
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
            .ok_or_else(|| AppError::Auth { message: "No ID token in response".into(), code: AuthErrorCode::TokenFailed })?;

        let id_token_verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&id_token_verifier, &nonce)
            .map_err(|e| AppError::Auth { message: format!("ID token validation failed: {e}"), code: AuthErrorCode::TokenInvalid })?;

        let external_sub = claims.subject().to_string();
        let external_email = claims.email().map(|e| e.to_string());
        let external_name = claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.to_string());

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
            let user = user_service
                .find_by_id(&identity.user_id)
                .await?
                .ok_or_else(|| AppError::Internal("Linked user not found".into()))?;
            if user.deactivated_at.is_some() {
                return Err(AppError::Auth {
                    message: "Account deactivated".into(),
                    code: AuthErrorCode::AccountDeactivated,
                });
            }
            return Ok((user, false));
        }

        if self.signups_match_email
            && let Some(ref email) = external_email
            && let Some(existing_user) = user_service.find_by_email(email).await?
        {
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
                .unwrap_or_else(|| "SSO User".to_string()),
            password_hash: String::new(),
            timezone: None,
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
