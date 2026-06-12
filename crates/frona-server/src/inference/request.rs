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
use super::registry::ModelProviderRegistry;
use super::tool_call::TaskEvent;

use crate::chat::message::models::Reasoning;

pub struct InferenceContext {
    pub user: User,
    pub agent: Agent,
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
        Self {
            user,
            agent,
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
}
