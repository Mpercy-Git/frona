use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

/// A grant that lets `recipient_id` view `chat_id` (owned by `owner_id`),
/// read-only — the recipient may not send messages, resolve HITL prompts, or
/// otherwise mutate the chat.
///
/// Uniqueness is enforced on `(chat_id, recipient_id)` so re-sharing is
/// idempotent (upsert). `owner_id` is denormalized from `chat.user_id` so
/// ownership checks and per-user cascade deletes don't need a join.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "chat_share")]
pub struct ChatShare {
    pub id: String,
    pub chat_id: String,
    pub owner_id: String,
    pub recipient_id: String,
    pub created_at: DateTime<Utc>,
}
