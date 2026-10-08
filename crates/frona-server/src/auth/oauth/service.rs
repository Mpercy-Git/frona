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

/// In-flight authorize flows, keyed by CSRF state: `(nonce_secret, Nonce, expiry)`.
///
/// Named because the bare type is dense enough to obscure the field it
/// describes, and it appears in both the struct and the constructor.
type PendingStates = Arc<Mutex<HashMap<String, (String, Nonce, chrono::DateTime<Utc>)>>>;

#[derive(Clone)]
pub struct OAuthService {
    authority: String,
    client_id: String,
    client_secret: String,
    scopes: Vec<String>,
    allow_unknown_email_verification: bool,
    signups_match_email: bool,
    // Entries are pruned on insert to prevent unbounded growth from abandoned
    // authorize flows.
    pending_states: PendingStates,
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
            pending_states: Arc::new(Mutex::new(HashMap::<
                String,
                (String, Nonce, chrono::DateTime<Utc>),
            >::new())),
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
        token_svc: &TokenService,
    ) -> Result<(User, bool), AppError> {
        let (_nonce_secret, nonce, _expiry) = self
            .pending_states
            .lock()
            .await
            .remove(state)
            .ok_or_else(|| AppError::Auth {
                message: "Invalid or expired SSO state".into(),
                code: AuthErrorCode::CsrfFailed,
            })?;

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

        let login = ExternalLogin {
            issuer: claims.issuer().as_str().to_string(),
            subject: claims.subject().to_string(),
            name: pick_name(
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
            ),
            email: claims.email().map(|e| ProviderEmail {
                address: AuthService::normalize_email(e.as_str()),
                verified: claims.email_verified() == Some(true),
            }),
        };

        self.check_email_claim(&login)?;

        if let Some(user) = self.known_user(user_service, &login).await? {
            return Ok((user, false));
        }

        // The userinfo endpoint is only worth a round trip when it could turn
        // up a verified address the ID token didn't carry (some IdPs emit
        // bare-sub ID tokens and only return email/name from userinfo).
        let userinfo = if self.needs_userinfo(&login) {
            match fetch_userinfo(&client, &http_client, &token_response, &login.subject).await {
                Ok(Some(info)) => Some(ExternalUserinfo {
                    email: info.email().map(|e| ProviderEmail {
                        address: AuthService::normalize_email(e.as_str()),
                        verified: info.email_verified() == Some(true),
                    }),
                    name: pick_name(
                        info.name().and_then(|n| n.get(None)).map(|n| n.as_str()),
                        info.given_name()
                            .and_then(|n| n.get(None))
                            .map(|n| n.as_str()),
                        info.family_name()
                            .and_then(|n| n.get(None))
                            .map(|n| n.as_str()),
                        info.preferred_username().map(|n| n.as_str()),
                    ),
                }),
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!(error = %e, "UserInfo fetch failed, falling back to ID token claims only");
                    None
                }
            }
        } else {
            None
        };

        self.sign_up(user_service, token_svc, &login, userinfo.as_ref())
            .await
    }

    /// Strict mode (`allow_unknown_email_verification = false`) turns away a
    /// sign-in whose email the provider has not vouched for. Permissive mode
    /// lets it in, but [`Self::sign_up`] never links such an address to an
    /// existing account.
    pub fn check_email_claim(&self, login: &ExternalLogin) -> Result<(), AppError> {
        match &login.email {
            Some(email) if !email.verified && !self.allow_unknown_email_verification => {
                Err(AppError::Auth {
                    message: "Email not verified by SSO provider".into(),
                    code: AuthErrorCode::EmailNotVerified,
                })
            }
            _ => Ok(()),
        }
    }

    /// The user this provider identity is already linked to, if any. The pair
    /// (issuer, subject) is the key: a subject from another issuer is a
    /// different person. A link recorded before issuers were tracked is
    /// adopted on its next sign-in.
    pub async fn known_user(
        &self,
        user_service: &UserService,
        login: &ExternalLogin,
    ) -> Result<Option<User>, AppError> {
        let Some(mut identity) = self
            .repo
            .find_identity(&login.issuer, &login.subject)
            .await?
        else {
            return Ok(None);
        };
        let Some(user) = user_service.find_by_id(&identity.user_id).await? else {
            tracing::warn!(
                identity_id = %identity.id,
                user_id = %identity.user_id,
                "Dropping orphaned SSO identity whose user no longer exists"
            );
            self.repo.delete(&identity.id).await?;
            return Ok(None);
        };
        if user.deactivated_at.is_some() {
            return Err(AppError::Auth {
                message: "Account deactivated".into(),
                code: AuthErrorCode::AccountDeactivated,
            });
        }
        if identity.issuer.is_none() {
            identity.issuer = Some(login.issuer.clone());
            identity.updated_at = Utc::now();
            self.repo.update(&identity).await?;
        }
        Ok(Some(user))
    }

    /// Whether [`Self::sign_up`] could use the userinfo endpoint: only when
    /// email matching is on and the ID token gave no verified address.
    pub fn needs_userinfo(&self, login: &ExternalLogin) -> bool {
        self.signups_match_email && !login.email.as_ref().is_some_and(|e| e.verified)
    }

    /// First sign-in for an identity the provider has not been seen before:
    /// link it to the existing account its verified email belongs to, or
    /// create a new account.
    ///
    /// An email is only evidence of ownership when the provider says it
    /// verified it, so only a verified address is matched against existing
    /// accounts. Anything we will not link to is also never created alongside
    /// an account holding the same address.
    pub async fn sign_up(
        &self,
        user_service: &UserService,
        token_svc: &TokenService,
        login: &ExternalLogin,
        userinfo: Option<&ExternalUserinfo>,
    ) -> Result<(User, bool), AppError> {
        let emails: Vec<&ProviderEmail> = login
            .email
            .iter()
            .chain(userinfo.and_then(|u| u.email.as_ref()))
            .collect();
        let external_email = emails.first().map(|e| e.address.clone());
        let external_name = login
            .name
            .clone()
            .or_else(|| userinfo.and_then(|u| u.name.clone()));

        let mut matched: Option<User> = None;
        if self.signups_match_email {
            for email in emails.iter().filter(|e| e.verified) {
                matched = user_service.find_by_email(&email.address).await?;
                if matched.is_some() {
                    break;
                }
            }
        }

        if let Some(existing_user) = matched {
            if existing_user.deactivated_at.is_some() {
                return Err(AppError::Auth {
                    message: "Account deactivated".into(),
                    code: AuthErrorCode::AccountDeactivated,
                });
            }
            // One person has one identity at a provider. An account already
            // bound to a different subject here is not theirs to take over.
            let already_bound = self
                .repo
                .find_identities_by_user(&existing_user.id)
                .await?
                .iter()
                .any(|i| {
                    i.external_sub != login.subject
                        && i.issuer.as_deref().is_none_or(|iss| iss == login.issuer)
                });
            if already_bound {
                return Err(conflict(
                    "This email already belongs to an account linked to a different SSO identity.",
                ));
            }
            let now = Utc::now();
            let identity = OAuthIdentity {
                id: crate::core::repository::new_id(),
                user_id: existing_user.id.clone(),
                issuer: Some(login.issuer.clone()),
                external_sub: login.subject.clone(),
                external_email: external_email.clone(),
                external_name,
                created_at: now,
                updated_at: now,
            };
            self.repo.create(&identity).await?;
            // Whoever held this account before the link, by password or by an
            // open session, does not keep it.
            token_svc
                .repo()
                .delete_by_user_id(&existing_user.id)
                .await?;
            return Ok((existing_user, false));
        }

        for email in &emails {
            if user_service.find_by_email(&email.address).await?.is_some() {
                return Err(conflict(
                    "An account with this email already exists, and the SSO provider has not verified \
                     the address (or linking by email is turned off). Sign in with your existing \
                     method or ask an admin to link it.",
                ));
            }
        }

        let now = Utc::now();
        let base_handle = if let Some(ref email) = external_email {
            AuthService::derive_handle_from_email(email)
        } else {
            format!("sso-{}", login.subject)
        };
        let handle = AuthService::generate_unique_handle(user_service, &base_handle).await?;

        let new_user = User {
            id: crate::core::repository::new_id(),
            handle,
            email: external_email
                .clone()
                .unwrap_or_else(|| format!("sso-{}@unknown", login.subject)),
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
            issuer: Some(login.issuer.clone()),
            external_sub: login.subject.clone(),
            external_email,
            external_name,
            created_at: now,
            updated_at: now,
        };
        self.repo.create(&identity).await?;

        Ok((user, true))
    }
}

fn conflict(message: &str) -> AppError {
    AppError::Auth {
        message: message.to_string(),
        code: AuthErrorCode::AccountConflict,
    }
}

/// An email address the provider asserted, and whether it vouched for it
/// (`email_verified == true`). An absent or false claim is not verification.
#[derive(Debug, Clone)]
pub struct ProviderEmail {
    pub address: String,
    pub verified: bool,
}

/// What the provider asserted about the person signing in, once the ID token's
/// signature, nonce and issuer have been checked.
#[derive(Debug, Clone)]
pub struct ExternalLogin {
    pub issuer: String,
    pub subject: String,
    pub name: Option<String>,
    pub email: Option<ProviderEmail>,
}

/// The same, from the provider's userinfo endpoint.
#[derive(Debug, Clone)]
pub struct ExternalUserinfo {
    pub email: Option<ProviderEmail>,
    pub name: Option<String>,
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
