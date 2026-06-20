use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::agent::service::AgentService;
use crate::agent::skill::service::SkillService;
use crate::agent::task::service::TaskService;
use crate::auth::UserService;
use crate::chat::broadcast::BroadcastService;
use crate::chat::command::{CommandContext, CommandOutcome, CommandRegistry};
use crate::chat::message::models::{Message, MessageCommand, MessageRole};
use crate::chat::service::ChatService;
use crate::chat::session::ChatSessionContext;
use crate::core::config::Config;
use crate::core::error::AppError;
use crate::core::state::ActiveSessions;
use crate::credential::vault::service::VaultService;
use crate::inference::conversation::{ConversationBuilder, DefaultConversationBuilder};
use crate::inference::hitl::{HitlOutcome, HitlResponse, ResolveOutcome};
use crate::inference::request::{InferenceContext, InferenceRequest, InferenceResponse};
use crate::inference::tool_call::ToolStatus;
use crate::memory::service::MemoryService;
use crate::policy::service::PolicyService;
use crate::agent::prompt::PromptLoader;
use crate::storage::StorageService;
use crate::tool::manager::ToolManager;
use crate::tool::mcp::McpServerService;
use crate::tool::registry::ToolFilter;

pub struct AgentLoopOutcome {
    /// What inference produced (Completed text, Cancelled, ExternalToolPending,
    /// or Handled when a command short-circuited the turn).
    pub inference: InferenceResponse,
    /// The in-flight agent message that the reply gets written into. Already
    /// reflects any mutations a command handler made (e.g. `agent_id` swap
    /// from `SwitchAgentCommand`). Callers pass this into the terminal-write
    /// APIs (`complete_agent_message`, `cancel_agent_message`, etc.) instead
    /// of fetching by id, so the handler's mutations land in a single write.
    pub response: Message,
}

/// Field typing mirrors AppState: bare types for services that derive `Clone`
/// internally (their fields are already `Arc`-wrapped), explicit `Arc<T>` for
/// services holding non-Clone state (`OnceLock`, `RwLock`, large config).
pub struct Harness {
    pub(crate) chat_service: ChatService,
    pub(crate) user_service: UserService,
    pub(crate) storage_service: StorageService,
    pub(crate) agent_service: AgentService,
    pub(crate) memory_service: MemoryService,
    pub(crate) skill_service: SkillService,
    pub(crate) task_service: TaskService,
    pub(crate) vault_service: VaultService,
    pub(crate) mcp_service: Arc<McpServerService>,
    pub(crate) tool_manager: Arc<ToolManager>,
    pub(crate) policy_service: PolicyService,
    pub(crate) broadcast_service: BroadcastService,
    pub(crate) active_sessions: ActiveSessions,
    pub(crate) shutdown_token: CancellationToken,
    pub(crate) prompts: PromptLoader,
    pub(crate) config: Arc<Config>,
    pub(crate) commands: Arc<CommandRegistry>,
}

impl Harness {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chat_service: ChatService,
        user_service: UserService,
        storage_service: StorageService,
        agent_service: AgentService,
        memory_service: MemoryService,
        skill_service: SkillService,
        task_service: TaskService,
        vault_service: VaultService,
        mcp_service: Arc<McpServerService>,
        tool_manager: Arc<ToolManager>,
        policy_service: PolicyService,
        broadcast_service: BroadcastService,
        active_sessions: ActiveSessions,
        shutdown_token: CancellationToken,
        prompts: PromptLoader,
        config: Arc<Config>,
    ) -> Self {
        let mut registry = CommandRegistry::new();
        crate::chat::command::builtin::register_all(&mut registry);
        let commands = Arc::new(registry);

        Self {
            chat_service,
            user_service,
            storage_service,
            agent_service,
            memory_service,
            skill_service,
            task_service,
            vault_service,
            mcp_service,
            tool_manager,
            policy_service,
            broadcast_service,
            active_sessions,
            shutdown_token,
            prompts,
            config,
            commands,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn(
        &self,
        user_id: &str,
        chat_id: &str,
        message_id: &str,
        cancel_token: CancellationToken,
        builder: Box<dyn ConversationBuilder>,
        tool_filters: &[ToolFilter],
        command_context_registry: Option<Arc<CommandRegistry>>,
    ) {
        let outcome = self
            .run_loop(
                user_id,
                chat_id,
                message_id,
                cancel_token,
                builder,
                tool_filters,
                command_context_registry,
            )
            .await;
        self.finalize(message_id, user_id, outcome).await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_loop(
        &self,
        user_id: &str,
        chat_id: &str,
        message_id: &str,
        cancel_token: CancellationToken,
        builder: Box<dyn ConversationBuilder>,
        tool_filters: &[ToolFilter],
        command_context_registry: Option<Arc<CommandRegistry>>,
    ) -> Result<AgentLoopOutcome, AppError> {
        let mut chat = self
            .chat_service
            .find_chat(chat_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;

        // `message_id` is the AGENT response placeholder. The user message is
        // separate. Signal/system-only chats may have no user-role message at
        // all — then there's nothing to dispatch and we go straight to inference.
        let request = self
            .chat_service
            .get_stored_messages(chat_id)
            .await?
            .into_iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User));

        let mut response = self.chat_service.get_message(user_id, message_id).await?;

        let builder_system_prompt = builder.system_prompt();

        let mut session = ChatSessionContext::build(
            self,
            user_id,
            chat.clone(),
            cancel_token.clone(),
            builder,
        )
        .await?;

        let mut prompt_override: Option<String> = None;
        if let Some(mut request) = request
            && matches!(request.role, MessageRole::User)
            && let Some(MessageCommand::Command { name, args }) = request.command.clone()
        {
            // Per-call registry wins over the default so callers can override
            // a built-in name within their own context.
            let user = self
                .user_service
                .find_by_id(user_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;

            let cmd = match command_context_registry.as_ref().and_then(|r| r.get(&name)) {
                Some(c) => Some(c),
                None => self.commands.resolve(&name, self, &user).await,
            };
            let cmd = cmd.ok_or_else(|| {
                AppError::NotFound(format!("command '{name}' not registered for this chat"))
            })?;

            // `response` is always written via the terminal API at end-of-turn,
            // so no snapshot is needed for it — only chat/request.
            let chat_snapshot = chat.clone();
            let request_snapshot = request.clone();

            let mut cmd_ctx = CommandContext {
                harness: self,
                session: &mut session,
                user: &user,
                chat: &mut chat,
                request: &mut request,
                response: &mut response,
            };

            let outcome = cmd.run(&args, &mut cmd_ctx).await;

            if chat != chat_snapshot {
                let _ = self.chat_service.save_chat(&chat).await;
            }
            if request != request_snapshot {
                let _ = self.chat_service.save_updated_message(&request).await;
            }

            match outcome {
                Ok(CommandOutcome::Prompt(rendered)) => {
                    prompt_override = Some(rendered);
                }
                Ok(CommandOutcome::Message(text)) => {
                    response.content = text;
                    let _ = self
                        .chat_service
                        .complete_agent_message(response.clone())
                        .await;
                    return Ok(AgentLoopOutcome {
                        inference: InferenceResponse::Handled,
                        response,
                    });
                }
                Ok(CommandOutcome::End) => {
                    let _ = self
                        .chat_service
                        .cancel_agent_message(response.clone())
                        .await;
                    return Ok(AgentLoopOutcome {
                        inference: InferenceResponse::Handled,
                        response,
                    });
                }
                Err(e) => {
                    response.content = format!("Command failed: {e}");
                    let _ = self
                        .chat_service
                        .complete_agent_message(response.clone())
                        .await;
                    return Ok(AgentLoopOutcome {
                        inference: InferenceResponse::Handled,
                        response,
                    });
                }
            }
        }

        let ChatSessionContext {
            mut system_prompt,
            model_group,
            mut rig_history,
            registry,
            mut tool_registry,
            tool_ctx,
            ..
        } = session;

        if let Some(extra) = builder_system_prompt {
            let trimmed = extra.trim();
            if !trimmed.is_empty() {
                system_prompt.push_str("\n\n");
                system_prompt.push_str(trimmed);
            }
        }

        for filter in tool_filters {
            tool_registry.apply_filter(filter);
        }

        // Swap only the model's view; the persisted `Message.content` is untouched.
        if let Some(rendered) = prompt_override
            && let Some(last_user) = rig_history
                .iter_mut()
                .rev()
                .find(|m| matches!(m, rig_core::completion::Message::User { .. }))
        {
            *last_user = rig_core::completion::Message::user(rendered);
        }

        let inference = crate::inference::inference(InferenceRequest {
            registry,
            model_group,
            system_prompt,
            history: rig_history,
            tool_registry,
            ctx: tool_ctx,
            cancel_token,
            chat_service: self.chat_service.clone(),
            message_id: message_id.to_string(),
        })
        .await?;

        Ok(AgentLoopOutcome { inference, response })
    }

    pub async fn resume(
        &self,
        user_id: &str,
        chat_id: &str,
        message_id: &str,
    ) -> Result<(), AppError> {
        let cancel_token = self.active_sessions.register(chat_id).await;
        let builder = Box::new(DefaultConversationBuilder {
            user_service: self.user_service.clone(),
            storage_service: self.storage_service.clone(),
            agent_service: self.agent_service.clone(),
        });
        self.run_turn(user_id, chat_id, message_id, cancel_token, builder, &[], None)
            .await;
        self.active_sessions.remove(chat_id).await;
        Ok(())
    }

    /// Does NOT spawn a resume — the caller dispatches via
    /// `state.task_executor.resume_or_notify(...)` when `should_resume`.
    pub async fn resolve_and_resume(
        &self,
        tool_call_id: &str,
        response: HitlResponse,
    ) -> Result<ResolveOutcome, AppError> {
        let te = self
            .chat_service
            .get_tool_call(tool_call_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("tool_call {tool_call_id}")))?;

        let hitl = te
            .hitl
            .as_ref()
            .ok_or_else(|| AppError::Validation(format!("tool_call {tool_call_id} has no HITL")))?;

        if matches!(hitl.status, ToolStatus::Resolved | ToolStatus::Denied) {
            return Ok(ResolveOutcome::AlreadyResolved);
        }

        let tool = self
            .tool_manager
            .find_tool_for_resume(&te.name)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "no tool registered to handle resume for '{}'",
                    te.name
                ))
            })?;

        let chat = self
            .chat_service
            .find_chat(&te.chat_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("chat {}", te.chat_id)))?;
        let user = self
            .user_service
            .find_by_id(&chat.user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", chat.user_id)))?;
        let agent = self
            .agent_service
            .find_by_id(&chat.agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("agent {}", chat.agent_id)))?;
        let event_tx = self.broadcast_service.create_event_sender(
            &user.id,
            &te.chat_id,
            chat.space_id.clone(),
        );
        let ctx = InferenceContext::new(
            user.clone(),
            agent,
            chat.clone(),
            event_tx,
            self.shutdown_token.clone(),
            CancellationToken::new(),
        );

        let request = hitl.request.clone();
        let outcome = tool
            .on_resume(&te.name, &request, response.clone(), &ctx)
            .await?;

        let resolved_message = match outcome {
            HitlOutcome::Resolved(text) => {
                self.chat_service
                    .resolve_tool_call_with_hitl_response(tool_call_id, Some(text), Some(response))
                    .await?
            }
            HitlOutcome::Denied(text) => {
                self.chat_service
                    .deny_tool_call_with_hitl_response(tool_call_id, Some(text), Some(response))
                    .await?
            }
        };

        let message_response = match resolved_message {
            crate::chat::service::ToolResolveResult::Changed(m)
            | crate::chat::service::ToolResolveResult::AlreadyResolved(m) => m,
        };

        self.broadcast_service.send(crate::chat::broadcast::BroadcastEvent {
            user_id: user.id.clone(),
            chat_id: Some(te.chat_id.clone()),
            space_id: chat.space_id.clone(),
            kind: crate::chat::broadcast::BroadcastEventKind::Inference(
                crate::inference::tool_loop::InferenceEventKind::Resume {
                    message: message_response,
                },
            ),
        });

        let did_flip = self
            .chat_service
            .mark_message_executing(&te.message_id)
            .await
            .unwrap_or(false);

        Ok(ResolveOutcome::Resolved {
            should_resume: did_flip,
            user_id: user.id.clone(),
            chat_id: te.chat_id.clone(),
            message_id: te.message_id.clone(),
            task_id: chat.task_id.clone(),
        })
    }

    pub async fn resume_all(self: &Arc<Self>) {
        let executing: Vec<Message> = self.chat_service.find_executing_chat_messages().await;
        if executing.is_empty() {
            return;
        }
        tracing::info!(
            count = executing.len(),
            "Resuming interrupted chats from previous run"
        );
        for msg in executing {
            let this = Arc::clone(self);
            let chat_id = msg.chat_id.clone();
            let msg_id = msg.id.clone();
            tokio::spawn(async move {
                let user_id = match this.chat_service.find_chat(&chat_id).await {
                    Ok(Some(chat)) => chat.user_id,
                    _ => {
                        tracing::error!(chat_id = %chat_id, "Failed to find chat for resume");
                        return;
                    }
                };
                if let Err(e) = this.resume(&user_id, &chat_id, &msg_id).await {
                    tracing::error!(error = %e, chat_id = %chat_id, "Failed to resume chat");
                }
            });
        }
    }

    async fn finalize(
        &self,
        message_id: &str,
        user_id: &str,
        outcome: Result<AgentLoopOutcome, AppError>,
    ) {
        match outcome {
            Ok(AgentLoopOutcome { inference, mut response }) => match inference {
                InferenceResponse::Completed {
                    text,
                    attachments,
                    reasoning,
                    ..
                } => {
                    response.content = text;
                    response.attachments = attachments;
                    response.reasoning = reasoning;
                    let _ = self.chat_service.complete_agent_message(response).await;
                }
                InferenceResponse::Cancelled(text) => {
                    response.content = text;
                    let _ = self.chat_service.cancel_agent_message(response).await;
                }
                InferenceResponse::ExternalToolPending { tool_calls, .. } => {
                    let _ = self
                        .chat_service
                        .pause_agent_message(
                            response,
                            crate::inference::tool_loop::PauseReason::Hitl,
                            tool_calls,
                        )
                        .await;
                }
                InferenceResponse::Handled => {
                    // Command dispatch already wrote/cancelled the response.
                }
            },
            Err(e) => {
                tracing::warn!(message_id, error = %e, "agent loop failed");
                // Best-effort: fetch the response for the failure event.
                if let Ok(msg) = self.chat_service.get_message(user_id, message_id).await {
                    let _ = self
                        .chat_service
                        .fail_agent_message(msg, e.to_string())
                        .await;
                }
            }
        }
    }
}
