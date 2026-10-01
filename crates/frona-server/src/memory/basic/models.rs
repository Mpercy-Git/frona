use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use frona_derive::Entity;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum MemorySourceType {
    Chat,
    Agent,
    Space,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "memory")]
pub struct Memory {
    pub id: String,
    pub source_type: MemorySourceType,
    pub source_id: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "memory_entry")]
pub struct MemoryEntry {
    pub id: String,
    pub agent_id: String,
    pub user_id: Option<String>,
    pub content: String,
    pub source_chat_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
