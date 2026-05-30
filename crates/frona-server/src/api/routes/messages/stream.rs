use axum::extract::{Path, State};
use axum::Json;
use rig_core::completion::Message as RigMessage;

use crate::chat::broadcast::{BroadcastEventKind, EventSender};
use crate::chat::message::models::SendMessageRequest;
use crate::chat::service::ChatService;
use crate::credential::presign::presign_response;
use crate::inference::conversation::{
    ConversationBuilder, ConversationContext, DefaultConversationBuilder, build_user_message,
};
use crate::inference::request::{InferenceRequest, InferenceResponse};
use crate::chat::message::models::MessageResponse;

use super::super::super::error::ApiError;
use super::super::super::middleware::auth::AuthUser;
use crate::core::state::AppState;

fn spawn_inference(
    req: InferenceRequest,
) -> tokio::task::JoinHandle<Result<InferenceResponse, crate::core::error::AppError>> {
    tokio::spawn(async move { crate::inference::inference(req).await })
}

async fn handle_inference_result(
    result: Result<Result<InferenceResponse, crate::core::error::AppError>, tokio::task::JoinError>,
    chat_service: &ChatService,
    message_id: &str,
    event_sender: &EventSender,
) {
    match result {
        Ok(Ok(response)) => match response {
            InferenceResponse::Completed { text, attachments, reasoning, .. } => {
                let _ = chat_service
                    .complete_agent_message(message_id, text, attachments, reasoning)
                    .await;
            }
            InferenceResponse::Cancelled(text) => {
                let _ = chat_service.cancel_agent_message(message_id, text).await;
            }
            InferenceResponse::ExternalToolPending {
                tool_calls, ..
            } => {
                for te in tool_calls {
                    event_sender.send_kind(BroadcastEventKind::ToolCallCreated { tool_call: te });
                }
            }
        },
        Ok(Err(e)) => {
            tracing::error!(error = %e, "Inference failed");
            let _ = chat_service.fail_agent_message(message_id).await;
        }
        Err(e) => {
            tracing::error!(error = %e, "Inference task panicked");
            let _ = chat_service.fail_agent_message(message_id).await;
        }
    }
}

pub(crate) async fn stream_message(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let chat = state
        .chat_service
        .get_chat(&auth.user_id, &chat_id)
        .await
        .map_err(ApiError::from)?;

    let pending_tool = state.chat_service
        .find_pending_tool_call(&chat_id)
        .await
        .map_err(ApiError::from)?;

    let cancel_token = state.active_sessions.register(&chat_id).await;
    let builder = Box::new(DefaultConversationBuilder {
        user_service: state.user_service.clone(),
        storage_service: state.storage_service.clone(),
    });
    let mut ctx = crate::chat::session::ChatSessionContext::build(
        &state, &auth.user_id, chat, cancel_token, builder,
    )
    .await
    .map_err(ApiError::from)?;

    if let Some(compaction_group) = state.compaction_model_group() {
        let max_output = ctx.model_group.max_tokens.unwrap_or(8192) as usize;
        if let Err(e) = state.memory_service.compact_chat_if_needed(
            &chat_id,
            &ctx.chat.agent_id,
            &ctx.system_prompt,
            &ctx.model_group.main.model_id,
            ctx.model_group.context_window,
            max_output,
            &compaction_group,
        ).await {
            tracing::warn!(error = %e, "Chat compaction failed, continuing without compaction");
        }
    }

    let stored_messages = state.chat_service.get_stored_messages(&chat_id).await?;
    let (chat_summary, context_messages) = state
        .memory_service
        .get_conversation_context(&chat_id)
        .await
        .unwrap_or((None, stored_messages));

    let mut rig_history = Vec::new();
    if let Some(summary) = &chat_summary {
        rig_history.push(RigMessage::user(format!(
            "[Previous conversation summary]\n{summary}"
        )));
        rig_history.push(RigMessage::assistant(
            "Understood. I have context from our previous conversation. How can I help?",
        ));
    }
    let conv_builder = DefaultConversationBuilder {
        user_service: state.user_service.clone(),
        storage_service: state.storage_service.clone(),
    };
    let conv_ctx = ConversationContext {
        agent_id: ctx.chat.agent_id.clone(),
        model_ref: ctx.model_group.main.clone(),
        user_id: auth.user_id.clone(),
    };
    let tool_calls = state.chat_service
        .get_tool_calls(&ctx.chat.id)
        .await
        .unwrap_or_default();
    rig_history.extend(conv_builder.build(&context_messages, &tool_calls, &conv_ctx).await);
    ctx.rig_history = rig_history;

    let user_content = req.content;
    let agent_id = ctx.chat.agent_id.clone();
    let needs_title = ctx.chat.title.is_none();

    let crate::chat::session::ChatSessionContext {
        system_prompt, model_group, rig_history, registry, tool_registry,
        mut tool_ctx, cancel_token, ..
    } = ctx;

    let event_sender = tool_ctx.event_tx.clone();

    if let Some(pending_te) = pending_tool {
        let mut user_response = state
            .chat_service
            .create_stream_user_message(&auth.user_id, &chat_id, &user_content, vec![])
            .await
            .map_err(ApiError::from)?;

        presign_response(&state.presign_service, &mut user_response, &auth.user_id, &auth.handle).await;

        let resolve_result = state
            .chat_service
            .resolve_tool_call(&pending_te.id, Some(user_content))
            .await
            .map_err(ApiError::from)?;

        let resolved_msg = resolve_result.into_message();

        let executing_msg = state.chat_service
            .find_executing_message_for_chat(&chat_id)
            .await
            .map_err(ApiError::from)?;

        let agent_msg_id = match executing_msg {
            Some(msg) => msg.id,
            None => {
                let msg = state.chat_service
                    .create_executing_agent_message(&chat_id, &agent_id)
                    .await
                    .map_err(ApiError::from)?;
                msg.id
            }
        };

        let pending_conv_builder = DefaultConversationBuilder {
            user_service: state.user_service.clone(),
            storage_service: state.storage_service.clone(),
        };
        let pending_conv_ctx = ConversationContext {
            agent_id: agent_id.clone(),
            model_ref: model_group.main.clone(),
            user_id: auth.user_id.clone(),
        };

        let chat_service = state.chat_service.clone();
        let active_sessions = state.active_sessions.clone();
        let chat_id_clone = chat_id.clone();

        event_sender.send_kind(BroadcastEventKind::ToolResolved { message: resolved_msg });

        let still_pending = state
            .chat_service
            .has_pending_tools_for_message(&agent_msg_id)
            .await
            .unwrap_or(false);

        if !still_pending {
            tokio::spawn(async move {
                let stored_messages = match chat_service.get_stored_messages(&chat_id_clone).await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!(chat_id = %chat_id_clone, error = %e, "failed to load stored messages for resume; aborting");
                        return;
                    }
                };
                let tool_calls = match chat_service.get_tool_calls(&chat_id_clone).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!(chat_id = %chat_id_clone, error = %e, "failed to load tool calls for resume; aborting");
                        return;
                    }
                };
                let rig_history = pending_conv_builder.build(&stored_messages, &tool_calls, &pending_conv_ctx).await;

                let handle = spawn_inference(InferenceRequest {
                    registry, model_group, system_prompt,
                    history: rig_history, tool_registry,
                    ctx: tool_ctx, cancel_token,
                    chat_service: chat_service.clone(),
                    message_id: agent_msg_id.clone(),
                });

                let result = handle.await;

                handle_inference_result(
                    result, &chat_service, &agent_msg_id, &event_sender,
                ).await;
                active_sessions.remove(&chat_id_clone).await;
            });
        }

        Ok(Json(user_response))
    } else {
        let attachments = req.attachments.clone();

        for att in &attachments {
            let resolved = crate::inference::conversation::resolve_attachment_path(
                att, &state.user_service, &state.storage_service,
            ).await;
            if !tool_ctx.file_paths.contains(&resolved) {
                tool_ctx.file_paths.push(resolved);
            }
        }

        let mut user_response = state
            .chat_service
            .create_stream_user_message(&auth.user_id, &chat_id, &user_content, req.attachments)
            .await
            .map_err(ApiError::from)?;

        presign_response(&state.presign_service, &mut user_response, &auth.user_id, &auth.handle).await;

        let agent_msg = state.chat_service
            .create_executing_agent_message(&chat_id, &agent_id)
            .await
            .map_err(ApiError::from)?;
        let agent_msg_id = agent_msg.id.clone();

        let chat_service = state.chat_service.clone();
        let active_sessions = state.active_sessions.clone();
        let msg_user_service = state.user_service.clone();
        let msg_storage_service = state.storage_service.clone();
        let chat_id_clone = chat_id.clone();

        tokio::spawn(async move {
            if needs_title {
                let svc = chat_service.clone();
                let cid = chat_id_clone.clone();
                let aid = agent_id.clone();
                let content = user_content.clone();
                let es = event_sender.clone();
                tokio::spawn(async move {
                    match svc.generate_title(&cid, &aid, &content).await {
                        Ok(title) => {
                            es.send_kind(BroadcastEventKind::Title { title });
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Title generation failed");
                        }
                    }
                });
            }

            let user_rig_msg = build_user_message(
                &user_content,
                &attachments,
                &msg_user_service,
                &msg_storage_service,
            ).await;
            let mut full_history = rig_history;
            full_history.push(user_rig_msg);

            let handle = spawn_inference(InferenceRequest {
                registry, model_group, system_prompt,
                history: full_history, tool_registry,
                ctx: tool_ctx, cancel_token,
                chat_service: chat_service.clone(),
                message_id: agent_msg_id.clone(),
            });

            let result = handle.await;

            handle_inference_result(
                result, &chat_service, &agent_msg_id, &event_sender,
            ).await;
            active_sessions.remove(&chat_id_clone).await;
        });

        Ok(Json(user_response))
    }
}
