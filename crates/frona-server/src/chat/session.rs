use rig_core::completion::Message as RigMessage;
pub use tokio_util::sync::CancellationToken;

use crate::agent::skill::resolver::Skill;
use crate::chat::broadcast::EventSender;
use crate::chat::command::render::render_skill;
use crate::chat::message::models::{Message, MessageCommand, MessageRole};
use crate::chat::models::Chat;
use crate::chat::service::AgentConfig;
use crate::core::error::AppError;
use crate::agent::harness::Harness;
use crate::inference::config::ModelGroup;
use crate::inference::conversation::{ConversationBuilder, ConversationContext, resolve_attachment_path};
use crate::inference::ModelProviderRegistry;
use crate::tool::registry::AgentToolRegistry;
use crate::tool::InferenceContext;

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
}

impl ChatSessionContext {
    pub async fn build(
        harness: &Harness,
        user_id: &str,
        chat: Chat,
        cancel_token: CancellationToken,
        builder: Box<dyn ConversationBuilder>,
    ) -> Result<Self, AppError> {
        let event_sender: EventSender = harness
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

        let skills = harness
            .skill_service
            .list(&user.handle, &agent.handle, agent_config.skills.as_deref())
            .await;

        // Load task early so `build_agent_registry` can register
        // task-domain tools in the same pass.
        let task = if let Some(ref task_id) = chat.task_id {
            harness.task_service.find_by_id(task_id).await.ok().flatten()
        } else {
            None
        };
        let task_in_progress = task.as_ref().is_some_and(|t|
            !matches!(t.kind, crate::agent::task::models::TaskKind::Cron { .. })
            && matches!(t.status,
                crate::agent::task::models::TaskStatus::Pending
                | crate::agent::task::models::TaskStatus::InProgress
            )
        );
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
            crate::tool::registry::build_agent_summaries(
                harness,
                user_id,
                &chat.agent_id,
            )
            .await;

        let mcp_servers: Vec<(String, String)> = if harness.config.mcp.bridge_mode {
            let servers = harness.mcp_service.list_for_user(user_id).await.unwrap_or_default();
            let allowed_handles: std::collections::HashSet<String> = allowed_tool_groups
                .iter()
                .filter_map(|id| {
                    id.strip_prefix("mcp:")
                        .map(|handle| handle.to_string())
                })
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

        let mut system_prompt = match harness
            .memory_service
            .build_augmented_system_prompt(
                &agent_config.system_prompt,
                &chat.agent_id,
                &agent.handle,
                user_id,
                &user.handle,
                chat.space_id.as_deref(),
                &skills,
                &agent_summaries,
                &agent_config.identity,
                &mcp_servers,
                &resolved_tz,
            )
            .await
        {
            Ok(prompt) => prompt,
            Err(e) => {
                tracing::warn!(error = %e, agent_id = %chat.agent_id, "Failed to build augmented system prompt, using base");
                agent_config.system_prompt.clone()
            }
        };

        let model_group = harness
            .chat_service
            .provider_registry()
            .resolve_model_group(&agent_config.model_group)?;

        let stored_messages = harness.chat_service.get_stored_messages(&chat.id).await?;
        let tool_calls = harness.chat_service
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
        //      synthetic acknowledgements for `/clear`, `/compact`, etc. —
        //      user-facing chrome, not conversation the model needs.
        let stored_messages = transform_for_commands(stored_messages, &skills);

        // Cron is already filtered from `task_in_progress`: TASK.md would prompt
        // complete_task → status=Completed → cron stops firing forever.
        if task_in_progress
            && let Some(task_prompt) = harness.prompts.read("TASK.md")
        {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&task_prompt);
        }

        if task_in_progress {
            tool_registry.apply_filter(
                &crate::tool::registry::ToolFilter::DenyList(&["create_recurring_task"]),
            );
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
            let tz: chrono_tz::Tz = resolved_tz
                .parse()
                .unwrap_or(chrono_tz::UTC);
            let fmt = "%Y-%m-%d %H:%M:%S %Z";
            let mut items = vec![
                ("created_at".into(), task.created_at.with_timezone(&tz).format(fmt).to_string()),
            ];
            if let Some(run_at) = task.run_at {
                items.push(("scheduled_at".into(), run_at.with_timezone(&tz).format(fmt).to_string()));
            }
            items.push(("now".into(), chrono::Utc::now().with_timezone(&tz).format(fmt).to_string()));
            crate::agent::prompt::append_tagged_section(
                &mut system_prompt,
                "task_time",
                None,
                &items,
            );
        }

        let rig_history = builder.build(&stored_messages, &tool_calls, &conv_ctx).await;

        let registry = harness.chat_service.provider_registry().clone();

        let mut file_paths = Vec::new();
        for msg in &stored_messages {
            for att in &msg.attachments {
                let resolved = resolve_attachment_path(att, &harness.user_service, &harness.storage_service).await;
                if !file_paths.contains(&resolved) {
                    file_paths.push(resolved);
                }
            }
        }

        let mut tool_ctx = InferenceContext::new(user, agent, chat.clone(), event_sender, harness.shutdown_token.clone(), cancel_token.clone());
        tool_ctx.file_paths = file_paths;
        tool_ctx.task = task;

        let vault_env = harness
            .vault_service
            .hydrate_chat_env_vars(user_id, &chat.id, &chat.agent_id)
            .await
            .unwrap_or_default();
        if !vault_env.is_empty() {
            let mut vault_vars = tool_ctx.vault_env_vars.write().await;
            vault_vars.extend(vault_env);
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
