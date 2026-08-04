use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

/// What a share grants the recipient. `Use` (chat with / run the agent, but
/// not edit it) is the only level today; `View`/`Edit` are reserved for future
/// expansion without a schema change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "lowercase")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum ShareLevel {
    Use,
}

/// A grant that lets `recipient_id` use `agent_id` (owned by `owner_id`).
///
/// Uniqueness is enforced on `(agent_id, recipient_id)` so re-sharing is
/// idempotent (upsert). `owner_id` is denormalized from `agent.user_id` so
/// ownership checks and per-user cascade deletes don't need a join.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "agent_share")]
pub struct AgentShare {
    pub id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub recipient_id: String,
    pub level: ShareLevel,
    /// When true, the recipient's runs of this agent may use the credentials
    /// the owner has granted the agent (the owner's durable bindings for
    /// `Principal::agent(agent_id)`). Off by default — Phase 2 opt-in. Defaults
    /// so existing rows deserialize.
    #[serde(default)]
    pub delegate_credentials: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
