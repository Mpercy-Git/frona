use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "policy")]
pub struct Policy {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub description: String,
    pub policy_text: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolStatus {
    pub id: String,
    pub group: String,
    pub enabled: bool,
    pub editable: bool,
}

#[derive(Debug, Clone)]
pub enum PolicyAction {
    InvokeTool { tool_name: String, tool_group: String },
    /// `target_handle` builds the Cedar UID; `target_agent_id` (UUID) is for internal lookups.
    DelegateTask { target_agent_id: String, target_handle: crate::core::Handle },
    SendMessage { target_agent_id: String, target_handle: crate::core::Handle },
    ReceiveSignal {
        connector_id: String,
        channel_handle: crate::core::Handle,
        sender: PolicyContact,
        paired_addresses: Vec<String>,
    },
    /// Deny here + `ReceiveSignal` allow falls back to signal-mode inference; both deny means discard.
    ReceiveMessage {
        connector_id: String,
        channel_handle: crate::core::Handle,
        sender: PolicyContact,
        paired_addresses: Vec<String>,
    },
    ListUsers,
    ManageUsers { target_user_id: String },
}

#[derive(Debug, Clone)]
pub struct PolicyContact {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub address: String,
    pub addresses: Vec<String>,
}

impl PolicyContact {
    pub fn unresolved(user_id: &str, address: &str) -> Self {
        Self {
            id: format!("unresolved:{}", address),
            user_id: user_id.to_string(),
            name: String::new(),
            address: address.to_string(),
            addresses: if address.is_empty() {
                Vec::new()
            } else {
                vec![address.to_string()]
            },
        }
    }

    pub fn from_contact(c: &crate::contact::models::Contact, address: &str) -> Self {
        Self {
            id: c.id.clone(),
            user_id: c.user_id.clone(),
            name: c.name.clone(),
            address: address.to_string(),
            addresses: [c.phone.clone(), c.email.clone()]
                .into_iter()
                .flatten()
                .collect(),
        }
    }
}

impl PolicyAction {
    pub fn cedar_action_name(&self) -> &'static str {
        match self {
            PolicyAction::InvokeTool { .. } => "invoke_tool",
            PolicyAction::DelegateTask { .. } => "delegate_task",
            PolicyAction::SendMessage { .. } => "send_message",
            PolicyAction::ReceiveSignal { .. } => "receive_signal",
            PolicyAction::ReceiveMessage { .. } => "receive_message",
            PolicyAction::ListUsers => "list_users",
            PolicyAction::ManageUsers { .. } => "manage_users",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthorizationDecision {
    pub allowed: bool,
    pub diagnostics: String,
}

impl AuthorizationDecision {
    pub fn allow() -> Self {
        Self {
            allowed: true,
            diagnostics: String::new(),
        }
    }

    pub fn deny(diagnostics: String) -> Self {
        Self {
            allowed: false,
            diagnostics,
        }
    }

    pub fn is_denied(&self) -> bool {
        !self.allowed
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePolicyRequest {
    pub policy_text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePolicyRequest {
    pub policy_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub policy_text: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Policy> for PolicyResponse {
    fn from(p: Policy) -> Self {
        Self {
            id: p.id,
            name: p.name,
            description: p.description,
            policy_text: p.policy_text,
            enabled: p.enabled,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PolicyResource {
    Tool { id: String, group: String },
    ToolGroup { group: String },
}

impl PolicyResource {
    pub fn label(&self) -> &str {
        match self {
            PolicyResource::Tool { id, .. } => id,
            PolicyResource::ToolGroup { group } => group,
        }
    }
}

