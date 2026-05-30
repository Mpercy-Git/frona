use tokio_util::sync::CancellationToken;

use crate::chat::broadcast::{BroadcastEventKind, EventSender};
use crate::chat::session::ChatSessionContext;
use crate::core::state::AppState;
use crate::core::error::AppError;
use crate::inference::conversation::{ConversationBuilder, DefaultConversationBuilder};
use crate::inference::request::{InferenceRequest, InferenceResponse};
use crate::tool::registry::ToolFilter;

pub struct AgentLoopOutcome {
    pub response: InferenceResponse,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_agent_turn(
    state: &AppState,
    user_id: &str,
    chat_id: &str,
    message_id: &str,
    cancel_token: CancellationToken,
    builder: Box<dyn ConversationBuilder>,
    tool_filters: &[ToolFilter],
    event_sender: Option<&EventSender>,
) {
    let outcome = run_agent_loop(
        state, user_id, chat_id, message_id, cancel_token, builder, tool_filters,
    )
    .await;
    finalize_agent_outcome(state, message_id, outcome, event_sender).await;
}

pub async fn finalize_agent_outcome(
    state: &AppState,
    message_id: &str,
    outcome: Result<AgentLoopOutcome, AppError>,
    event_sender: Option<&EventSender>,
) {
    match outcome {
        Ok(AgentLoopOutcome { response }) => match response {
            InferenceResponse::Completed { text, attachments, reasoning, .. } => {
                let _ = state
                    .chat_service
                    .complete_agent_message(message_id, text, attachments, reasoning)
                    .await;
            }
            InferenceResponse::Cancelled(text) => {
                let _ = state.chat_service.cancel_agent_message(message_id, text).await;
            }
            InferenceResponse::ExternalToolPending { tool_calls, .. } => {
                if let Some(es) = event_sender {
                    for te in tool_calls {
                        es.send_kind(BroadcastEventKind::ToolCallCreated { tool_call: te });
                    }
                }
            }
        },
        Err(e) => {
            tracing::warn!(message_id, error = %e, "agent loop failed");
            let _ = state.chat_service.fail_agent_message(message_id).await;
        }
    }
}

pub async fn run_agent_loop(
    state: &AppState,
    user_id: &str,
    chat_id: &str,
    message_id: &str,
    cancel_token: CancellationToken,
    builder: Box<dyn ConversationBuilder>,
    tool_filters: &[ToolFilter],
) -> Result<AgentLoopOutcome, AppError> {
    let chat = state
        .chat_service
        .find_chat(chat_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;

    let builder_system_prompt = builder.system_prompt();

    let ChatSessionContext {
        mut system_prompt, model_group, rig_history, registry,
        mut tool_registry, tool_ctx, ..
    } = ChatSessionContext::build(
        state, user_id, chat, cancel_token.clone(), builder,
    ).await?;

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

    let response = crate::inference::inference(InferenceRequest {
        registry,
        model_group,
        system_prompt,
        history: rig_history,
        tool_registry,
        ctx: tool_ctx,
        cancel_token,
        chat_service: state.chat_service.clone(),
        message_id: message_id.to_string(),
    })
    .await?;

    Ok(AgentLoopOutcome { response })
}

/// Resume all interactive chats that were mid-inference when the server stopped.
/// Only resumes non-task chats — task chats are handled by the task executor.
pub async fn resume_all_chats(state: &AppState) {
    let executing = state.chat_service.find_executing_chat_messages().await;

    if executing.is_empty() {
        return;
    }

    tracing::info!(count = executing.len(), "Resuming interrupted chats from previous run");

    for msg in executing {
        let state = state.clone();
        let chat_id = msg.chat_id.clone();
        let msg_id = msg.id.clone();
        tokio::spawn(async move {
            let user_id = match state.chat_service.find_chat(&chat_id).await {
                Ok(Some(chat)) => chat.user_id,
                _ => {
                    tracing::error!(chat_id = %chat_id, "Failed to find chat for resume");
                    return;
                }
            };
            if let Err(e) = resume_agent_loop(&state, &user_id, &chat_id, &msg_id).await {
                tracing::error!(error = %e, chat_id = %chat_id, "Failed to resume chat");
            }
        });
    }
}

/// Resume an interrupted chat (after external tool resolution, child task completion, etc.).
/// Runs the agent loop, saves the result, and broadcasts to the user via the unified SSE stream.
pub async fn resume_agent_loop(
    state: &AppState,
    user_id: &str,
    chat_id: &str,
    message_id: &str,
) -> Result<(), AppError> {
    let cancel_token = state.active_sessions.register(chat_id).await;
    let space_id = state
        .chat_service
        .get_chat(user_id, chat_id)
        .await
        .ok()
        .and_then(|c| c.space_id);
    let event_sender = state
        .broadcast_service
        .create_event_sender(user_id, chat_id, space_id);

    let builder = Box::new(DefaultConversationBuilder {
        user_service: state.user_service.clone(),
        storage_service: state.storage_service.clone(),
    });
    run_agent_turn(
        state, user_id, chat_id, message_id, cancel_token, builder, &[], Some(&event_sender),
    )
    .await;

    state.active_sessions.remove(chat_id).await;
    Ok(())
}
