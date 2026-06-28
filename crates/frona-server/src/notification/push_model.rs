use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

/// A stored Web Push subscription for a user.
/// One user can have multiple (phone, laptop, desktop).
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "push_subscription")]
pub struct PushSubscription {
    pub id: String,
    pub user_id: String,
    /// The push endpoint URL (unique per browser/device).
    pub endpoint: String,
    /// Expiration time in milliseconds since epoch, or None if no expiration.
    pub expiration_time: Option<i64>,
    /// The P-256 ECDSA public key (base64url-encoded).
    pub p256dh_key: String,
    /// The authentication secret (base64url-encoded).
    pub auth_secret: String,
    pub created_at: DateTime<Utc>,
}