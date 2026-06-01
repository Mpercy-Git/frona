use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "lowercase")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum ToolStatus {
    Pending,
    Resolved,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(tag = "type", content = "data")]
#[surreal(crate = "surrealdb::types", tag = "type", content = "data")]
pub enum MessageTool {
    HumanInTheLoop {
        reason: String,
        debugger_url: String,
        status: ToolStatus,
        response: Option<String>,
    },
    Question {
        question: String,
        options: Vec<String>,
        status: ToolStatus,
        response: Option<String>,
    },
    TaskCompletion {
        task_id: String,
        chat_id: Option<String>,
        status: crate::agent::task::models::TaskStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        deliverables: Vec<crate::storage::Attachment>,
    },
    TaskDeferred {
        task_id: String,
        delay_minutes: u32,
        reason: String,
    },
    VaultApproval {
        query: String,
        reason: String,
        env_var_prefix: Option<String>,
        status: ToolStatus,
        response: Option<String>,
    },
    ServiceApproval {
        action: String,
        manifest: serde_json::Value,
        previous_manifest: Option<serde_json::Value>,
        status: ToolStatus,
        response: Option<String>,
    },
}

impl MessageTool {
    pub fn tool_status(&self) -> Option<&ToolStatus> {
        match self {
            Self::HumanInTheLoop { status, .. }
            | Self::Question { status, .. }
            | Self::VaultApproval { status, .. }
            | Self::ServiceApproval { status, .. } => Some(status),
            Self::TaskCompletion { .. } | Self::TaskDeferred { .. } => None,
        }
    }

    pub fn tool_response(&self) -> Option<&str> {
        match self {
            Self::HumanInTheLoop { response, .. }
            | Self::Question { response, .. }
            | Self::VaultApproval { response, .. }
            | Self::ServiceApproval { response, .. } => response.as_deref(),
            Self::TaskCompletion { .. } | Self::TaskDeferred { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "tool_call")]
pub struct ToolCall {
    pub id: String,
    pub chat_id: String,
    pub message_id: String,
    pub turn: u32,
    pub provider_call_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: String,
    pub success: bool,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_data: Option<MessageTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_text: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    pub id: String,
    pub chat_id: String,
    pub message_id: String,
    pub turn: u32,
    pub provider_call_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: String,
    pub success: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_data: Option<MessageTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_text: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<ToolCall> for ToolCallResponse {
    fn from(te: ToolCall) -> Self {
        Self {
            id: te.id,
            chat_id: te.chat_id,
            message_id: te.message_id,
            turn: te.turn,
            provider_call_id: te.provider_call_id,
            name: te.name,
            arguments: te.arguments,
            result: te.result,
            success: te.success,
            duration_ms: te.duration_ms,
            tool_data: te.tool_data,
            system_prompt: te.system_prompt,
            description: te.description,
            turn_text: te.turn_text,
            created_at: te.created_at,
        }
    }
}
