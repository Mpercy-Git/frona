use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "oauth_identity")]
pub struct OAuthIdentity {
    pub id: String,
    pub user_id: String,
    /// The provider that issued `external_sub`. A subject is only unique within
    /// its issuer, so the pair is the identity. Absent on rows created before
    /// issuers were recorded; the next sign-in fills it in.
    #[serde(default)]
    pub issuer: Option<String>,
    pub external_sub: String,
    pub external_email: Option<String>,
    pub external_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
