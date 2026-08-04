use std::sync::Arc;

use rig_core::completion::Message as RigMessage;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::agent::models::Agent;
use crate::agent::task::models::Task;
use crate::chat::broadcast::EventSender;
use crate::chat::models::Chat;
use crate::auth::User;
use crate::tool::registry::AgentToolRegistry;

use super::config::ModelGroup;
use super::usage::UsageService;
use super::registry::ModelProviderRegistry;
use super::tool_call::TaskEvent;

use crate::chat::message::models::Reasoning;

pub struct InferenceContext {
    pub user: User,
    pub agent: Agent,
    /// Handle of the agent's owner. Equals `user.handle` for an owned agent;
    /// for a shared agent (runner ≠ owner) it is the owner's handle, so
    /// definition/execution-scoped lookups (skills, agent workspace, sandbox
    /// policy) resolve under the owner. Runner-scoped concerns (vault, memory,
    /// ephemeral token) keep using `user`.
    pub agent_owner_handle: crate::core::Handle,
    /// When this is a shared agent whose owner opted into credential
    /// delegation, the owner's user id — the account whose agent credential
    /// bindings this run may use. `None` for owned agents or shares without
    /// delegation. See `request_credentials` and the session-build hydration.
    pub delegated_credential_owner: Option<String>,
    pub chat: Chat,
    pub task: Option<Task>,
    pub event_tx: EventSender,
    pub vault_env_vars: Arc<RwLock<Vec<(String, String)>>>,
    /// Resolved filesystem paths for files shared in this chat (from message attachments).
    pub file_paths: Vec<String>,
    pub shutdown_token: CancellationToken,
    /// User-initiated cancellation token — tools should check/use this to abort early.
    pub cancel_token: CancellationToken,
}

impl InferenceContext {
    pub fn new(
        user: User,
        agent: Agent,
        chat: Chat,
        event_tx: EventSender,
        shutdown_token: CancellationToken,
        cancel_token: CancellationToken,
    ) -> Self {
        // Default to the runner as owner; `with_agent_owner_handle` overrides
        // this for shared agents. Correct for the common owned-agent case.
        let agent_owner_handle = user.handle.clone();
        Self {
            user,
            agent,
            agent_owner_handle,
            delegated_credential_owner: None,
            chat,
            task: None,
            event_tx,
            vault_env_vars: Arc::new(RwLock::new(Vec::new())),
            file_paths: Vec::new(),
            shutdown_token,
            cancel_token,
        }
    }

    pub fn with_task(mut self, task: Task) -> Self {
        self.task = Some(task);
        self
    }

    /// Override the agent-owner handle (used when the agent was shared with the
    /// running user, so their runs resolve the owner's skills/workspace/policy).
    pub fn with_agent_owner_handle(mut self, handle: crate::core::Handle) -> Self {
        self.agent_owner_handle = handle;
        self
    }

    /// Mark that this run may use the given owner's delegated agent credentials.
    pub fn with_delegated_credential_owner(mut self, owner_id: Option<String>) -> Self {
        self.delegated_credential_owner = owner_id;
        self
    }
}

pub struct InferenceRequest {
    pub registry: ModelProviderRegistry,
    pub model_group: ModelGroup,
    pub system_prompt: String,
    pub history: Vec<RigMessage>,
    pub tool_registry: AgentToolRegistry,
    pub ctx: InferenceContext,
    pub cancel_token: CancellationToken,
    pub chat_service: crate::chat::service::ChatService,
    pub message_id: String,
    pub usage_service: UsageService,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum InferenceResponse {
    Completed {
        text: String,
        attachments: Vec<crate::storage::Attachment>,
        lifecycle_event: Option<TaskEvent>,
        reasoning: Option<Reasoning>,
    },
    Cancelled(String),
    ExternalToolPending {
        turn_text: String,
        tool_calls: Vec<crate::inference::tool_call::ToolCallResponse>,
        system_prompt: Option<String>,
    },
    /// The harness already wrote the response for this turn — `finalize`
    /// should leave the placeholder agent message alone. Used by the
    /// `Command` dispatch path (`/clear`, `/compact`, `/title`, etc.) which
    /// either completes the placeholder inline or wipes it as a side effect.
    Handled,
}
