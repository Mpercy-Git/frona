use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;
use crate::core::Handle;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "user")]
pub struct User {
    pub id: String,
    /// Globally-unique. Rendered as "Username" in auth UIs.
    pub handle: Handle,
    pub email: String,
    pub name: String,
    pub password_hash: String,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    #[surreal(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub deactivated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn resolve_timezone(stored: Option<&str>, server_default: &str) -> String {
    stored
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| server_default.to_string())
}

impl User {
    pub fn resolved_timezone(&self, server_default: &str) -> String {
        resolve_timezone(self.timezone.as_deref(), server_default)
    }
}

pub const ADMINS_GROUP: &str = "admins";

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "user_group")]
pub struct UserGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub built_in: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub identifier: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub handle: String,
    pub email: String,
    pub name: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Debug, Serialize, Default)]
pub struct UserPermissions {
    pub list_users: bool,
}

#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: String,
    pub handle: Handle,
    pub email: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_setup: Option<bool>,
    #[serde(default)]
    pub permissions: UserPermissions,
}

#[derive(Debug, Deserialize)]
pub struct UpdateHandleRequest {
    pub handle: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub handle: Handle,
    pub email: String,
    pub exp: usize,
    pub iat: usize,
    pub token_id: String,
    pub token_type: String,
    pub principal: crate::core::Principal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extensions: Option<serde_json::Value>,
}
