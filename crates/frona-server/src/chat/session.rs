use std::sync::Arc;

use rig_core::completion::Message as RigMessage;
pub use tokio_util::sync::CancellationToken;

use crate::agent::harness::Harness;
use crate::chat::broadcast::EventSender;
use crate::chat::message::models::{Message, MessageRole};
use crate::chat::models::Chat;
use crate::chat::service::AgentConfig;
use crate::core::error::AppError;
use crate::inference::ModelGroup;
use crate::inference::ModelProviderRegistry;
use crate::inference::conversation::{
    ConversationBuilder, ConversationContext, ModelConversationHistory, resolve_attachment_path,
};
use crate::tool::InferenceContext;
use crate::tool::registry::{AgentToolRegistry, ToolFilter};

pub struct ChatSessionContext {
    pub chat: Chat,
    pub agent_config: AgentConfig,
    pub system_prompt: String,
    pub model_group: ModelGroup,
    pub rig_history: Vec<RigMessage>,
    pub registry: ModelProviderRegistry,
    pub(crate) model_history: ModelConversationHistory,
    pub(crate) input_token_estimate: Option<crate::inference::context::InputTokenEstimate>,
    pub(crate) builder: Arc<dyn ConversationBuilder>,
    pub(crate) tool_filters: Vec<ToolFilter>,

    pub tool_registry: AgentToolRegistry,
    pub tool_ctx: InferenceContext,
    pub cancel_token: CancellationToken,
    /// The most recent user message as stored, captured while the history was
    /// loaded here. Callers that need it (slash-command dispatch in
    /// `Harness::run_loop`) would otherwise re-read every message in the chat
    /// just to look at the last one. Raw - before the skill/command rewrites
    /// applied to the model history - because the caller persists edits to it.
    pub last_user_message: Option<Message>,
}

impl ChatSessionContext {
    pub fn agent_context_scope(&self) -> crate::agent::context::AgentContextScope<'_> {
        self.tool_ctx.agent_context_scope()
    }

    pub async fn build(
        harness: &Harness,
        user_id: &str,
        chat: Chat,
        cancel_token: CancellationToken,
        builder: Arc<dyn ConversationBuilder>,
        tool_filters: &[ToolFilter],
    ) -> Result<Self, AppError> {
        Self::build_for_response(
            harness,
            user_id,
            chat,
            None,
            cancel_token,
            builder,
            tool_filters,
        )
        .await
    }

    /// Agent switches reuse the already-initialized turn.
    pub(crate) async fn build_for_response(
        harness: &Harness,
        user_id: &str,
        chat: Chat,
        response_id: Option<&str>,
        cancel_token: CancellationToken,
        builder: Arc<dyn ConversationBuilder>,
        tool_filters: &[ToolFilter],
    ) -> Result<Self, AppError> {
        let event_sender: EventSender =
            harness
                .broadcast_service
                .create_event_sender(user_id, &chat.id, chat.space_id.clone());
        let agent_config = harness
            .chat_service
            .resolve_agent_config(&chat.agent_id)
            .await?;

        let agent = harness
            .agent_service
            .find_by_id(&chat.agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;

        let user = harness
            .user_service
            .find_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound("User not found".into()))?;

        // For a shared agent (runner != owner), definition-scoped lookups must
        // resolve under the OWNER: skills, agent workspace, sandbox policy.
        // For an owned agent this is just the runner's own handle.
        let agent_owner_handle = if agent.user_id == user_id {
            user.handle.clone()
        } else {
            harness.user_service.handle_of(&agent.user_id).await?
        };

        // If this is a shared agent whose owner opted into credential
        // delegation, the recipient's run may use the owner's agent credentials.
        let delegated_credential_owner = harness
            .agent_service
            .credential_delegation_owner(&agent, user_id)
            .await
            .unwrap_or(None);

        // Load task early so `build_agent_registry` can register
        // task-domain tools in the same pass.
        let task = if let Some(ref task_id) = chat.task_id {
            Some(
                harness
                    .task_service
                    .find_by_id(task_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Task".into()))?,
            )
        } else {
            None
        };
        let task_in_progress = task.as_ref().is_some_and(|t| {
            !matches!(t.kind, crate::agent::task::models::TaskKind::Cron { .. })
                && matches!(
                    t.status,
                    crate::agent::task::models::TaskStatus::Pending
                        | crate::agent::task::models::TaskStatus::InProgress
                )
        });
        let task_ctx = if task_in_progress {
            task.clone().map(|t| crate::tool::manager::TaskToolContext {
                task: t,
                storage_service: harness.storage_service.clone(),
                prompts: harness.prompts.clone(),
                chat_service: harness.chat_service.clone(),
                task_service: harness.task_service.clone(),
            })
        } else {
            None
        };

        let mut tool_registry = harness
            .tool_manager
            .build_agent_registry(user_id, &agent, &harness.policy_service, task_ctx)
            .await;

        // `send_message` initiates a new user-facing message; it only makes sense
        // when the agent is firing autonomously in its heartbeat chat. In a task
        // chat the delivery channel is `complete_task.result`; in a normal chat
        // the agent already replies by streaming. Allowing it elsewhere lets the
        // model duplicate work or, worse, satisfy a "send a reminder" instruction
        // via `send_message` and then leave `complete_task.result` empty against
        // a non-nullable schema.
        let in_heartbeat_chat = agent.heartbeat_chat_id.as_deref() == Some(&chat.id);
        if !in_heartbeat_chat {
            tool_registry.deny(&["send_message"]);
        }

        // Resolved whatever the bridge setting is: every run wants to know which
        // MCP systems it can reach.
        let allowed_handles: std::collections::HashSet<String> = tool_registry
            .tool_groups()
            .iter()
            .filter_map(|id| id.strip_prefix("mcp:").map(|handle| handle.to_string()))
            .collect();
        let mcp_handles: Vec<String> = if allowed_handles.is_empty() {
            Vec::new()
        } else {
            harness
                .mcp_service
                .list_for_user(user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.status == crate::tool::mcp::models::McpServerStatus::Running)
                .filter(|s| allowed_handles.contains(s.handle.as_str()))
                .map(|s| s.handle.to_string())
                .collect()
        };

        harness
            .mcp_service
            .register_agent_tool(&mut tool_registry, &harness.prompts);

        let registry = harness.chat_service.provider_registry().clone();
        // `resolve_model_group` accepts both a named group and an ad-hoc
        // "provider/model" reference; a missing named group falls back to primary.
        let model_group = if agent_config.model_group.contains('/') {
            registry.resolve_model_group(&agent_config.model_group)?
        } else {
            registry.resolve_with_fallback(
                &crate::inference::ModelRef(agent_config.model_group.clone().into()),
                &crate::inference::ModelRef::PRIMARY,
            )?
        };

        let loaded = harness
            .chat_service
            .compactor()
            .load_conversation(&chat.id)
            .await?;
        let agent_context = loaded.agent_context;
        let conversation_summary = loaded.summary;
        let mut stored_messages = loaded.messages;
        if let Some(response_id) = response_id {
            let response_index = stored_messages
                .iter()
                .position(|m| m.id == response_id)
                .ok_or_else(|| {
                    AppError::Conflict("Response is outside the active conversation".into())
                })?;
            stored_messages.truncate(response_index + 1);
        }
        let tool_calls = harness.chat_service.get_tool_calls(&chat.id).await?;
        // Captured before the history is built and rewritten.
        let last_user_message = stored_messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
            .cloned();

        // Cron is already filtered from `task_in_progress`: TASK.md would prompt
        // complete_task → status=Completed → cron stops firing forever.
        let task_prompt = task_in_progress
            .then(|| harness.prompts.read("TASK.md"))
            .flatten();

        if task_in_progress {
            tool_registry.apply_filter(&crate::tool::registry::ToolFilter::DenyList(&[
                "create_recurring_task",
            ]));
        }

        let model_ref = model_group.main.clone();
        let conv_ctx = ConversationContext {
            agent_id: chat.agent_id.clone(),
            model_ref,
            user_id: user_id.to_string(),
        };

        let mut model_history = builder
            .build(
                &stored_messages,
                &tool_calls,
                &conv_ctx,
                conversation_summary.as_deref(),
            )
            .await;
        model_history.set_summary_context(agent_context.as_deref());

        // Images can't go to a model that doesn't accept them - providers 404
        // the whole request. Only act when the catalog positively reports no
        // vision support (unknown models are left untouched to avoid regressing
        // capable ones). Preferred handling: transcribe the images with a
        // vision-capable model (an override "vision" group, else auto-selected)
        // and inline the text so the agent still gets the content. If no vision
        // model is available, strip the images so the turn still runs. Done on the
        // model history itself so compaction weighs what will actually be sent.
        let catalog_vision = harness
            .usage_service
            .model_supports_vision(&conv_ctx.model_ref);
        let effective_vision = crate::inference::vision::resolve_vision_capability(
            &conv_ctx.model_ref,
            &model_group.inference,
            catalog_vision,
        );
        if effective_vision == Some(false) {
            match crate::inference::vision::resolve_vision_model_group(
                &registry,
                &harness.usage_service,
            ) {
                Some(vision_group) => {
                    let img_msg_id = stored_messages
                        .iter()
                        .rev()
                        .find(|m| !m.attachments.is_empty())
                        .map(|m| m.id.clone())
                        .unwrap_or_default();
                    let n = crate::inference::vision::transcribe_images_in_history(
                        &mut model_history.history,
                        &vision_group,
                        &registry,
                        &harness.usage_service,
                        user_id,
                        &chat.agent_id,
                        &chat.id,
                        &img_msg_id,
                    )
                    .await;
                    if n > 0 {
                        tracing::info!(
                            agent_model = %conv_ctx.model_ref.as_str(),
                            vision_model = %vision_group.main.as_str(),
                            images = n,
                            "transcribed images for text-only agent model",
                        );
                    }
                }
                None => {
                    let n = crate::inference::conversation::strip_images_from_history(
                        &mut model_history.history,
                    );
                    if n > 0 {
                        tracing::info!(
                            model = %conv_ctx.model_ref.as_str(),
                            images = n,
                            "stripped image attachments (no vision model available)",
                        );
                    }
                }
            }
        }

        let mut file_paths = Vec::new();
        for msg in &stored_messages {
            for att in &msg.attachments {
                let resolved = resolve_attachment_path(
                    att,
                    user_id,
                    &harness.user_service,
                    &harness.storage_service,
                )
                .await;
                if !file_paths.contains(&resolved) {
                    file_paths.push(resolved);
                }
            }
        }

        let extra = builder.system_prompt();
        let extra = extra
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let additional_instructions: Vec<&str> =
            task_prompt.as_deref().into_iter().chain(extra).collect();
        let system_prompt = harness
            .chat_service
            .build_system_prompt(&agent_config, &user, &agent, &additional_instructions)
            .await?;

        let mut tool_ctx = InferenceContext::new(
            user,
            agent,
            chat.clone(),
            event_sender,
            harness.shutdown_token.clone(),
            cancel_token.clone(),
        )
        .with_agent_owner_handle(agent_owner_handle)
        .with_delegated_credential_owner(delegated_credential_owner.clone())
        .with_mcp_servers(mcp_handles);
        tool_ctx.file_paths = file_paths;
        tool_ctx.task = task;

        let mut vault_env = harness
            .vault_service
            .resolve_env(
                user_id,
                &crate::core::Principal::agent(&chat.agent_id),
                Some(&chat.id),
            )
            .await
            .unwrap_or_default();
        // Credential delegation: also load the owner's durable agent credentials.
        if let Some(ref owner_id) = delegated_credential_owner {
            let delegated = harness
                .vault_service
                .resolve_delegated_env(owner_id, &chat.agent_id, &chat.id)
                .await
                .unwrap_or_default();
            vault_env.extend(delegated);
        }
        if !vault_env.is_empty() {
            let mut vault_vars = tool_ctx.vault_env_vars.write().await;
            vault_vars.extend(vault_env);
        }

        let rig_history;

        for filter in tool_filters {
            tool_registry.apply_filter(filter);
        }
        let tools = crate::inference::tool_loop::to_rig_tool_definitions(
            tool_registry.definitions(),
            tool_registry.mcp_bridge_active(),
        );
        let compacted = harness
            .chat_service
            .compactor()
            .compact_chat(
                &chat.id,
                super::compactor::CompactionContext {
                    agent_context: tool_ctx.agent_context_scope(),
                    system_prompt: &system_prompt,
                    model_group: &model_group,
                    model_history: &model_history,
                    tools: &tools,
                },
            )
            .await?;
        let input_token_estimate = compacted.input_token_estimate;
        model_history = compacted.model_history;
        rig_history = model_history.history.clone();
        super::compactor::validate_initial_context(
            &rig_history,
            &system_prompt,
            &model_group,
            &tools,
            input_token_estimate,
        )?;

        Ok(Self {
            chat,
            agent_config,
            system_prompt,
            model_group,
            rig_history,
            registry,
            model_history,
            input_token_estimate,
            builder,
            tool_filters: tool_filters.to_vec(),

            tool_registry,
            tool_ctx,
            cancel_token,
            last_user_message,
        })
    }
}
