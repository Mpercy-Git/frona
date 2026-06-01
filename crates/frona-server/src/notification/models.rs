use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(tag = "type")]
#[surreal(crate = "surrealdb::types")]
pub enum NotificationData {
    App {
        app_handle: String,
        action: String,
    },
    Task {
        task_id: String,
    },
    Agent {
        agent_id: String,
        chat_id: String,
    },
    System {},
    Security {},
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", snake_case)]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "notification")]
pub struct Notification {
    pub id: String,
    pub user_id: String,
    pub data: NotificationData,
    pub level: NotificationLevel,
    pub title: String,
    pub body: String,
    pub read: bool,
    pub created_at: DateTime<Utc>,
}
