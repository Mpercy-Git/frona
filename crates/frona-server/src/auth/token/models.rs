use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;
use crate::core::Principal;

/// Scope of a token that may only wake one agent through
/// `POST /api/agents/{id}/trigger` (a doorbell, a webhook). Such a token is
/// refused everywhere else: `TokenService::validate` rejects it, and only
/// `TokenService::validate_agent_trigger` accepts it. That keeps a token sitting
/// in a third-party system's config from opening the owner's account.
pub const AGENT_TRIGGER_SCOPE: &str = "agent:trigger";

/// Scope of the short-lived code that carries a signed-in user from the main
/// origin to the apps origin (`GET /api/auth/apps/callback`). Refused by
/// `TokenService::validate`, so it cannot call any account API.
pub const APP_GATE_SCOPE: &str = "app:gate";

/// Scope of the `app_session` cookie the apps origin keeps. Like
/// [`APP_GATE_SCOPE`] it is refused by `TokenService::validate`; only the apps
/// proxy accepts it, through `TokenService::validate_app_session`.
pub const APP_SESSION_SCOPE: &str = "app:session";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", rename_all = "snake_case")]
pub enum TokenType {
    Access,
    Refresh,
    Pat,
    Ephemeral,
}

impl TokenType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenType::Access => "access",
            TokenType::Refresh => "refresh",
            TokenType::Pat => "pat",
            TokenType::Ephemeral => "ephemeral",
        }
    }

    pub fn is_stateless(&self) -> bool {
        matches!(self, TokenType::Ephemeral)
    }
}

impl std::fmt::Display for TokenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "api_token")]
pub struct ApiToken {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub token_type: TokenType,
    pub principal: Principal,
    pub scopes: Vec<String>,
    pub prefix: String,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub refresh_pair_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePatRequest {
    pub name: String,
    pub expires_in_days: Option<u64>,
    pub scopes: Option<Vec<String>>,
    #[serde(default)]
    pub principal: Option<Principal>,
}

#[derive(Debug, Serialize)]
pub struct PatResponse {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub token: String,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PatListItem {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
