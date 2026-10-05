use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(tag = "type")]
#[surreal(crate = "surrealdb::types")]
pub enum NotificationData {
    App { app_handle: String, action: String },
    Task { task_id: String },
    Agent { agent_id: String, chat_id: String },
    Channel { channel_id: String, action: String },
    CostReport { report_id: String },
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

/// What a notification is about, for deciding whether it is worth pushing to
/// the user's devices. Every notification still lands in the in-app list; the
/// category only gates the Web Push.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCategory {
    /// An agent is paused until the user authorises something.
    Approval,
    /// An agent reached out unprompted (`send_message`).
    AgentMessage,
    /// An agent finished replying in one of the user's own chats.
    ChatReply,
    /// An app, MCP server or channel failed or crashed.
    Failure,
    /// Routine app lifecycle and report updates (deployed, stopped, cost
    /// report ready).
    Activity,
}

/// Per-user choice of which categories are pushed to devices. A user with no
/// stored row gets [`NotificationPreferences::default`]; the row is written
/// whole, so every field is always present.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "notification_preferences")]
pub struct NotificationPreferences {
    /// Keyed by the owning user's id — one row per user. Both are set by the
    /// server, so clients may omit them.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    pub push_approval: bool,
    pub push_agent_message: bool,
    pub push_chat_reply: bool,
    pub push_failure: bool,
    pub push_activity: bool,
}

impl NotificationPreferences {
    pub fn defaults_for(user_id: &str) -> Self {
        Self {
            id: user_id.to_string(),
            user_id: user_id.to_string(),
            push_approval: true,
            push_agent_message: true,
            push_chat_reply: true,
            push_failure: true,
            push_activity: false,
        }
    }

    pub fn pushes(&self, category: NotificationCategory) -> bool {
        match category {
            NotificationCategory::Approval => self.push_approval,
            NotificationCategory::AgentMessage => self.push_agent_message,
            NotificationCategory::ChatReply => self.push_chat_reply,
            NotificationCategory::Failure => self.push_failure,
            NotificationCategory::Activity => self.push_activity,
        }
    }
}
