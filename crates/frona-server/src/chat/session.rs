use rig_core::completion::Message as RigMessage;
pub use tokio_util::sync::CancellationToken;

use crate::agent::harness::Harness;
use crate::agent::skill::resolver::Skill;
use crate::chat::broadcast::EventSender;
use crate::chat::command::render::render_skill;
use crate::chat::message::models::{Message, MessageCommand, MessageRole};
use crate::chat::models::Chat;
use crate::chat::service::AgentConfig;
use crate::core::error::AppError;
use crate::inference::ModelProviderRegistry;
use crate::inference::config::ModelGroup;
use crate::inference::conversation::{
    ConversationBuilder, ConversationContext, resolve_attachment_path,
};
use crate::tool::InferenceContext;
use crate::tool::registry::AgentToolRegistry;

pub struct ChatSessionContext {
    pub chat: Chat,
    pub agent_config: AgentConfig,
    pub system_prompt: String,
    pub model_group: ModelGroup,
    pub rig_history: Vec<RigMessage>,
    pub registry: ModelProviderRegistry,
    pub tool_registry: AgentToolRegistry,
    pub tool_ctx: InferenceContext,
    pub cancel_token: CancellationToken,
    /// The most recent user message as stored, captured while the history was
    /// loaded here. Callers that need it (slash-command dispatch in
    /// `Harness::run_loop`) would otherwise re-read every message in the chat
    /// just to look at the last one. Raw — before the skill/command rewrites
    /// applied to `rig_history` — because the caller persists edits to it.
    pub last_user_message: Option<Message>,
}

impl ChatSessionContext {
    pub async fn build(
        harness: &Harness,
        user_id: &str,
        chat: Chat,
        cancel_token: CancellationToken,
        builder: Box<dyn ConversationBuilder>,
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

        // For a shared agent (runner ≠ owner), definition-scoped lookups must
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

        let skills = harness
            .skill_service
            .list(&agent_owner_handle, &agent.handle, agent_config.skills.as_deref())
            .await;

        // Load task early so `build_agent_registry` can register
        // task-domain tools in the same pass.
        let task = if let Some(ref task_id) = chat.task_id {
            harness
                .task_service
                .find_by_id(task_id)
                .await
                .ok()
                .flatten()
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

        let allowed_tool_groups = tool_registry.tool_groups();

        let agent_summaries =
            crate::tool::registry::build_agent_summaries(harness, user_id, &chat.agent_id).await;

        let mcp_servers: Vec<(String, String)> = if harness.config.mcp.bridge_mode {
            let servers = harness
                .mcp_service
                .list_for_user(user_id)
                .await
                .unwrap_or_default();
            let allowed_handles: std::collections::HashSet<String> = allowed_tool_groups
                .iter()
                .filter_map(|id| id.strip_prefix("mcp:").map(|handle| handle.to_string()))
                .collect();
            servers
                .into_iter()
                .filter(|s| s.status == crate::tool::mcp::models::McpServerStatus::Running)
                .filter(|s| allowed_handles.contains(s.handle.as_str()))
                .map(|s| {
                    let desc = s.description.unwrap_or_else(|| s.display_name.clone());
                    (s.handle.to_string(), desc)
                })
                .collect()
        } else {
            Vec::new()
        };

        let resolved_tz = user.resolved_timezone(&harness.config.server.timezone);

        let mut system_prompt = crate::agent::prompt::build_augmented_system_prompt(
            &agent_config.system_prompt,
            &agent_config.identity,
            &harness.prompts,
            &harness.storage_service,
            &user.handle,
            &agent.handle,
            &skills,
            &agent_summaries,
            &mcp_servers,
            &resolved_tz,
        );

        let model_group = harness
            .chat_service
            .provider_registry()
            .resolve_model_group(&agent_config.model_group)?;

        let max_output = model_group
            .max_tokens
            .unwrap_or(model_group.inference.default_max_tokens) as usize;
        let loaded = harness
            .chat_service
            .compactor()
            .compact_chat(
                user_id,
                &chat.id,
                &chat.agent_id,
                &system_prompt,
                model_group.context_window,
                max_output,
            )
            .await?
            .conversation;
        let conversation_summary = loaded.summary;
        let stored_messages = loaded.messages;
        // Captured before the rewrites below, which the caller must not see.
        let last_user_message = stored_messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
            .cloned();
        let tool_calls = harness
            .chat_service
            .get_tool_calls(&chat.id)
            .await
            .unwrap_or_default();

        // Apply two slash-command transformations to the message list the
        // builder will see:
        //   1. For user messages with `command: Some(Skill { name, prompt })`,
        //      render the SKILL.md body and replace `content` with the
        //      `<skill ...>...</skill>` form. Persistent DB row is untouched.
        //   2. Drop assistant messages whose immediately-preceding message is
        //      a user message with `command: Some(Command { … })`. Those are
        //      synthetic acknowledgements for `/clear`, `/compact`, etc. -
        //      user-facing chrome, not conversation the model needs.
        let stored_messages = transform_for_commands(stored_messages, &skills);

        // Cron is already filtered from `task_in_progress`: TASK.md would prompt
        // complete_task → status=Completed → cron stops firing forever.
        if task_in_progress && let Some(task_prompt) = harness.prompts.read("TASK.md") {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&task_prompt);
        }

        if task_in_progress {
            tool_registry.apply_filter(&crate::tool::registry::ToolFilter::DenyList(&[
                "create_recurring_task",
            ]));
        }

        for te in &tool_calls {
            if let Some(sp) = &te.system_prompt {
                system_prompt.push_str("\n\n");
                system_prompt.push_str(sp);
            }
        }

        let model_ref = model_group.main.clone();
        let conv_ctx = ConversationContext {
            agent_id: chat.agent_id.clone(),
            model_ref,
            user_id: user_id.to_string(),
        };

        if let Some(ref task) = task {
            let tz: chrono_tz::Tz = resolved_tz.parse().unwrap_or(chrono_tz::UTC);
            let fmt = "%Y-%m-%d %H:%M:%S %Z";
            let mut items = vec![(
                "created_at".into(),
                task.created_at.with_timezone(&tz).format(fmt).to_string(),
            )];
            if let Some(run_at) = task.run_at {
                items.push((
                    "scheduled_at".into(),
                    run_at.with_timezone(&tz).format(fmt).to_string(),
                ));
            }
            items.push((
                "now".into(),
                chrono::Utc::now()
                    .with_timezone(&tz)
                    .format(fmt)
                    .to_string(),
            ));
            crate::agent::prompt::append_tagged_section(
                &mut system_prompt,
                "task_time",
                None,
                &items,
            );
        }

        let mut rig_history = builder
            .build(
                &stored_messages,
                &tool_calls,
                &conv_ctx,
                conversation_summary.as_deref(),
            )
            .await;

        let registry = harness.chat_service.provider_registry().clone();

        // Images can't go to a model that doesn't accept them — providers 404
        // the whole request. Only act when the catalog positively reports no
        // vision support (unknown models are left untouched to avoid regressing
        // capable ones). Preferred handling: transcribe the images with a
        // vision-capable model (an override "vision" group, else auto-selected)
        // and inline the text so the agent still gets the content. If no vision
        // model is available, strip the images so the turn still runs.
        let catalog_vision = harness.usage_service.model_supports_vision(&conv_ctx.model_ref);
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
                        &mut rig_history,
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
                        &mut rig_history,
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
                let resolved =
                    resolve_attachment_path(att, &harness.user_service, &harness.storage_service)
                        .await;
                if !file_paths.contains(&resolved) {
                    file_paths.push(resolved);
                }
            }
        }

        let mut tool_ctx = InferenceContext::new(
            user,
            agent,
            chat.clone(),
            event_sender,
            harness.shutdown_token.clone(),
            cancel_token.clone(),
        )
        .with_agent_owner_handle(agent_owner_handle)
        .with_delegated_credential_owner(delegated_credential_owner.clone());
        tool_ctx.file_paths = file_paths;
        tool_ctx.task = task;

        let mut vault_env = harness
            .vault_service
            .hydrate_chat_env_vars(user_id, &chat.id, &chat.agent_id)
            .await
            .unwrap_or_default();
        // Credential delegation: also load the owner's durable agent credentials.
        if let Some(ref owner_id) = delegated_credential_owner {
            let delegated = harness
                .vault_service
                .hydrate_delegated_env_vars(owner_id, &chat.agent_id, &chat.id)
                .await
                .unwrap_or_default();
            vault_env.extend(delegated);
        }
        if !vault_env.is_empty() {
            let mut vault_vars = tool_ctx.vault_env_vars.write().await;
            vault_vars.extend(vault_env);
        }

        {
            let mut mcx = crate::memory::service::MemoryContext::new(
                &mut system_prompt,
                &mut rig_history,
                &tool_ctx,
            );
            if let Err(e) = harness.memory_service.retrieve(&mut mcx).await {
                tracing::warn!(error = %e, "memory retrieve failed; continuing without memory block");
            }
        }

        Ok(Self {
            chat,
            agent_config,
            system_prompt,
            model_group,
            rig_history,
            registry,
            tool_registry,
            tool_ctx,
            cancel_token,
            last_user_message,
        })
    }
}

/// Rewrites Skill-command user messages to the rendered SKILL.md body for the
/// model's view of this turn. The persisted DB row is untouched.
fn transform_for_commands(messages: Vec<Message>, skills: &[Skill]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    for mut msg in messages {
        if matches!(msg.role, MessageRole::User)
            && let Some(MessageCommand::Skill { name, prompt }) = msg.command.clone()
            && let Some(rendered) = render_skill(&name, &prompt, skills)
        {
            msg.content = rendered;
        }

        out.push(msg);
    }
    out
}
