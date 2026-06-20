use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

use crate::Entity;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "chat")]
pub struct Chat {
    pub id: String,
    pub user_id: String,
    pub space_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    pub agent_id: String,
    pub title: Option<String>,
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_external_id: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChatRequest {
    pub space_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    pub agent_id: String,
    pub title: Option<String>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChatRequest {
    pub title: Option<String>,
    pub space_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub id: String,
    pub space_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_id: String,
    pub title: Option<String>,
    pub archived_at: Option<DateTime<Utc>>,
    pub channel_id: Option<String>,
    pub channel_external_id: Option<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Chat> for ChatResponse {
    fn from(chat: Chat) -> Self {
        Self {
            id: chat.id,
            space_id: chat.space_id,
            task_id: chat.task_id,
            agent_id: chat.agent_id,
            title: chat.title,
            archived_at: chat.archived_at,
            channel_id: chat.channel_id,
            channel_external_id: chat.channel_external_id,
            metadata: chat.metadata,
            created_at: chat.created_at,
            updated_at: chat.updated_at,
        }
    }
}
