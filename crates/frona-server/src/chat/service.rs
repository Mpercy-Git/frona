use crate::agent::config::parse_frontmatter;
use crate::agent::service::AgentService;
use crate::agent::workspace::AgentPromptLoader;
use crate::storage::StorageService;
use crate::db::repo::chats::SurrealChatRepo;
use crate::db::repo::messages::SurrealMessageRepo;
use crate::core::error::AppError;
use crate::core::metrics::InferenceMetricsContext;
use crate::core::template::render_template;
use crate::inference::ModelProviderRegistry;
use crate::auth::UserService;
use crate::inference::conversation::{ConversationBuilder, ConversationContext, DefaultConversationBuilder};
use crate::inference::text_inference;
use crate::inference::provider::ModelRef;
use crate::memory::service::MemoryService;
use crate::agent::prompt::PromptLoader;
use crate::core::repository::Repository;
use rig::completion::Message as RigMessage;

pub struct AgentConfig {
    pub system_prompt: String,
    pub model_group: String,
    pub skills: Option<Vec<String>>,
    pub sandbox_limits: Option<crate::core::config::SandboxLimits>,
    pub identity: std::collections::BTreeMap<String, String>,
}

use super::models::{ChatResponse, CreateChatRequest, UpdateChatRequest};
use super::message::models::{MessageResponse, MessageStatus, SendMessageRequest, MessageEvent, PaginatedMessagesResponse};
use super::message::models::{Message, MessageRole, Reasoning};
use super::message::repository::MessageRepository;
use super::models::Chat;
use super::repository::ChatRepository;
use crate::db::repo::tool_calls::ToolCallRepository;
use crate::inference::tool_call::{MessageTool, ToolCall, ToolCallResponse, ToolStatus};
pub enum ToolResolveResult {
    Changed(MessageResponse),
    AlreadyResolved(MessageResponse),
}

impl ToolResolveResult {
    pub fn into_message(self) -> MessageResponse {
        match self {
            Self::Changed(msg) | Self::AlreadyResolved(msg) => msg,
        }
    }
}

#[derive(Clone)]
pub struct ChatService {
    chat_repo: SurrealChatRepo,
    message_repo: SurrealMessageRepo,
    tool_call_repo: crate::db::repo::tool_calls::SurrealToolCallRepo,
    agent_service: AgentService,
    provider_registry: ModelProviderRegistry,
    storage_service: StorageService,
    user_service: UserService,
    memory_service: MemoryService,
    prompts: PromptLoader,
    broadcast: crate::chat::broadcast::BroadcastService,
}

impl ChatService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chat_repo: SurrealChatRepo,
        message_repo: SurrealMessageRepo,
        tool_call_repo: crate::db::repo::tool_calls::SurrealToolCallRepo,
        agent_service: AgentService,
        provider_registry: ModelProviderRegistry,
        storage_service: StorageService,
        user_service: UserService,
        memory_service: MemoryService,
        prompts: PromptLoader,
        broadcast: crate::chat::broadcast::BroadcastService,
    ) -> Self {
        Self {
            chat_repo,
            message_repo,
            tool_call_repo,
            agent_service,
            provider_registry,
            storage_service,
            user_service,
            memory_service,
            prompts,
            broadcast,
        }
    }

    fn broadcast_chat_entity(&self, chat: &Chat, action: crate::chat::broadcast::EntityAction) {
        self.broadcast.broadcast_entity_updated(
            &chat.user_id,
            "chat",
            &chat.id,
            action,
            chat.space_id.clone(),
            None,
        );
    }

    fn broadcast_message_entity(
        &self,
        msg: &Message,
        chat: &Chat,
        action: crate::chat::broadcast::EntityAction,
    ) {
        self.broadcast.broadcast_entity_updated(
            &chat.user_id,
            "message",
            &msg.id,
            action,
            chat.space_id.clone(),
            None,
        );
    }


    pub fn provider_registry(&self) -> &ModelProviderRegistry {
        &self.provider_registry
    }

    pub fn memory_service(&self) -> &MemoryService {
        &self.memory_service
    }

    pub async fn create_chat(
        &self,
        user_id: &str,
        req: CreateChatRequest,
    ) -> Result<ChatResponse, AppError> {
        let now = chrono::Utc::now();
        let chat = Chat {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            space_id: req.space_id,
            task_id: req.task_id,
            agent_id: req.agent_id,
            title: req.title,
            archived_at: None,
            channel_id: None,
            channel_external_id: None,
            metadata: req.metadata.unwrap_or_default(),
            created_at: now,
            updated_at: now,
        };

        let chat = self.chat_repo.create(&chat).await?;
        self.broadcast_chat_entity(&chat, crate::chat::broadcast::EntityAction::Created);
        Ok(chat.into())
    }

    pub async fn get_message(&self, user_id: &str, message_id: &str) -> Result<Message, AppError> {
        let msg = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        let chat = self
            .chat_repo
            .find_by_id(&msg.chat_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;
        if chat.user_id != user_id {
            return Err(AppError::Forbidden("Not your message".into()));
        }
        Ok(msg)
    }

    pub async fn get_chat(&self, user_id: &str, chat_id: &str) -> Result<Chat, AppError> {
        let chat = self
            .chat_repo
            .find_by_id(chat_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;

        if chat.user_id != user_id {
            return Err(AppError::Forbidden("Not your chat".into()));
        }

        Ok(chat)
    }

    pub async fn list_chats(&self, user_id: &str) -> Result<Vec<ChatResponse>, AppError> {
        let chats = self.chat_repo.find_by_user_id(user_id).await?;
        Ok(chats.into_iter().map(Into::into).collect())
    }

    pub async fn update_chat(
        &self,
        user_id: &str,
        chat_id: &str,
        req: UpdateChatRequest,
    ) -> Result<ChatResponse, AppError> {
        let mut chat = self.get_chat(user_id, chat_id).await?;

        if let Some(title) = req.title {
            chat.title = Some(title);
        }
        if let Some(space_id) = req.space_id {
            chat.space_id = Some(space_id);
        }
        if let Some(patch) = req.metadata {
            crate::core::metadata::apply_metadata_patch(&mut chat.metadata, patch);
        }
        chat.updated_at = chrono::Utc::now();

        let chat = self.chat_repo.update(&chat).await?;
        self.broadcast_chat_entity(&chat, crate::chat::broadcast::EntityAction::Updated);
        Ok(chat.into())
    }

    pub async fn upsert_channel_chat(
        &self,
        user_id: &str,
        space_id: &str,
        agent_id: &str,
        channel_id: &str,
        channel_external_id: &str,
        title: Option<&str>,
    ) -> Result<Chat, AppError> {
        if let Some(c) = self
            .chat_repo
            .find_by_channel_thread(channel_id, channel_external_id)
            .await?
        {
            return Ok(c);
        }
        let now = chrono::Utc::now();
        let chat = Chat {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            space_id: Some(space_id.to_string()),
            task_id: None,
            agent_id: agent_id.to_string(),
            title: title.map(|s| s.to_string()),
            archived_at: None,
            channel_id: Some(channel_id.to_string()),
            channel_external_id: Some(channel_external_id.to_string()),
            metadata: std::collections::BTreeMap::new(),
            created_at: now,
            updated_at: now,
        };
        let saved = self.chat_repo.create(&chat).await?;
        self.broadcast_chat_entity(&saved, crate::chat::broadcast::EntityAction::Created);
        Ok(saved)
    }

    pub async fn patch_chat_metadata(
        &self,
        chat_id: &str,
        patch: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Chat, AppError> {
        let mut chat = self
            .chat_repo
            .find_by_id(chat_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;
        crate::core::metadata::apply_metadata_patch(&mut chat.metadata, patch);
        chat.updated_at = chrono::Utc::now();
        let saved = self.chat_repo.update(&chat).await?;
        self.broadcast_chat_entity(&saved, crate::chat::broadcast::EntityAction::Updated);
        Ok(saved)
    }

    pub async fn update_message_metadata(
        &self,
        user_id: &str,
        message_id: &str,
        req: crate::chat::message::models::UpdateMessageRequest,
    ) -> Result<crate::chat::message::models::MessageResponse, AppError> {
        let mut msg = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        let chat = self
            .chat_repo
            .find_by_id(&msg.chat_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;
        if chat.user_id != user_id {
            return Err(AppError::Forbidden("Not your message".into()));
        }
        if let Some(patch) = req.metadata {
            crate::core::metadata::apply_metadata_patch(&mut msg.metadata, patch);
        }
        let saved = self.message_repo.update(&msg).await?;
        self.broadcast_message_entity(&saved, &chat, crate::chat::broadcast::EntityAction::Updated);
        Ok(saved.into())
    }

    pub async fn persist_inbound_message(&self, msg: &Message) -> Result<Message, AppError> {
        let saved = self.message_repo.create(msg).await?;
        if let Ok(Some(chat)) = self.chat_repo.find_by_id(&saved.chat_id).await {
            self.broadcast_message_entity(
                &saved,
                &chat,
                crate::chat::broadcast::EntityAction::Created,
            );
        }
        Ok(saved)
    }

    pub async fn patch_message_metadata(
        &self,
        message_id: &str,
        patch: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Message, AppError> {
        let mut msg = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        crate::core::metadata::apply_metadata_patch(&mut msg.metadata, patch);
        let saved = self.message_repo.update(&msg).await?;
        if let Ok(Some(chat)) = self.chat_repo.find_by_id(&saved.chat_id).await {
            self.broadcast_message_entity(&saved, &chat, crate::chat::broadcast::EntityAction::Updated);
        }
        Ok(saved)
    }

    pub async fn delete_chat(&self, user_id: &str, chat_id: &str) -> Result<(), AppError> {
        let chat = self.get_chat(user_id, chat_id).await?;
        self.chat_repo.delete(chat_id).await?;
        self.broadcast_chat_entity(&chat, crate::chat::broadcast::EntityAction::Deleted);
        Ok(())
    }

    pub async fn archive_chat(
        &self,
        user_id: &str,
        chat_id: &str,
    ) -> Result<ChatResponse, AppError> {
        let mut chat = self.get_chat(user_id, chat_id).await?;

        chat.archived_at = Some(chrono::Utc::now());
        chat.updated_at = chrono::Utc::now();
        let chat = self.chat_repo.update(&chat).await?;
        Ok(chat.into())
    }

    pub async fn unarchive_chat(
        &self,
        user_id: &str,
        chat_id: &str,
    ) -> Result<ChatResponse, AppError> {
        let mut chat = self.get_chat(user_id, chat_id).await?;

        chat.archived_at = None;
        chat.updated_at = chrono::Utc::now();
        let chat = self.chat_repo.update(&chat).await?;
        Ok(chat.into())
    }

    pub async fn list_archived_chats(
        &self,
        user_id: &str,
    ) -> Result<Vec<ChatResponse>, AppError> {
        let chats = self.chat_repo.find_archived_by_user_id(user_id).await?;
        Ok(chats.into_iter().map(Into::into).collect())
    }

    pub async fn send_message(
        &self,
        user_id: &str,
        chat_id: &str,
        req: SendMessageRequest,
    ) -> Result<Vec<MessageResponse>, AppError> {
        let chat = self.get_chat(user_id, chat_id).await?;

        let title_handle = if chat.title.is_none() {
            let svc = self.clone();
            let agent_id = chat.agent_id.clone();
            let user_content = req.content.clone();
            let cid = chat_id.to_string();
            Some(tokio::spawn(async move {
                if let Err(e) = svc
                    .generate_title(&cid, &agent_id, &user_content)
                    .await
                {
                    tracing::warn!(error = %e, "Title generation failed");
                }
            }))
        } else {
            None
        };

        let mut user_message_builder =
            Message::builder(chat_id, MessageRole::User, req.content.clone());
        if let Some(ref md) = req.metadata {
            user_message_builder = user_message_builder.metadata(md.clone());
        }
        let user_message = user_message_builder.build();
        let user_message = self.message_repo.create(&user_message).await?;

        let agent_config = self.resolve_agent_config(&chat.agent_id).await?;
        let system_prompt = agent_config.system_prompt;
        let model_group_name = agent_config.model_group;

        let stored_messages = self.message_repo.find_by_chat_id(chat_id).await?;
        let model_group = self.provider_registry.get_model_group(&model_group_name)?;
        let conv_builder = DefaultConversationBuilder {
            user_service: self.user_service.clone(),
            storage_service: self.storage_service.clone(),
        };
        let conv_ctx = ConversationContext {
            agent_id: chat.agent_id.clone(),
            model_ref: model_group.main.clone(),
            user_id: user_id.to_string(),
        };
        let tool_calls = self.get_tool_calls(chat_id).await?;
        let mut rig_history = conv_builder.build(&stored_messages, &tool_calls, &conv_ctx).await;

        rig_history.push(RigMessage::user(&req.content));
        let response_text = text_inference(
            &self.provider_registry,
            model_group,
            &system_prompt,
            rig_history,
            &InferenceMetricsContext::default(),
        )
        .await?;

        let assistant_message = Message::builder(chat_id, MessageRole::Agent, response_text)
            .agent_id(chat.agent_id.clone())
            .build();
        let assistant_message = self.message_repo.create(&assistant_message).await?;

        if let Some(handle) = title_handle {
            let _ = handle.await;
        }

        Ok(vec![user_message.into(), assistant_message.into()])
    }

    pub async fn list_messages(
        &self,
        user_id: &str,
        chat_id: &str,
    ) -> Result<Vec<MessageResponse>, AppError> {
        self.get_chat(user_id, chat_id).await?;

        let messages = self.message_repo.find_by_chat_id(chat_id).await?;
        let tool_calls = self.get_tool_calls(chat_id).await.unwrap_or_default();

        let mut te_map: std::collections::HashMap<String, Vec<ToolCallResponse>> =
            std::collections::HashMap::new();
        for te in tool_calls {
            te_map
                .entry(te.message_id.clone())
                .or_default()
                .push(te.into());
        }

        Ok(messages
            .into_iter()
            .map(|msg| {
                let id = msg.id.clone();
                let mut resp: MessageResponse = msg.into();
                if let Some(tes) = te_map.remove(&id) {
                    resp.tool_calls = tes;
                }
                resp
            })
            .collect())
    }

    pub async fn list_messages_paginated(
        &self,
        user_id: &str,
        chat_id: &str,
        before: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<chrono::DateTime<chrono::Utc>>,
        limit: u32,
    ) -> Result<PaginatedMessagesResponse, AppError> {
        self.get_chat(user_id, chat_id).await?;

        let fetch_limit = limit + 1;
        let mut messages = self
            .message_repo
            .find_by_chat_id_page(chat_id, before, after, fetch_limit)
            .await?;

        let has_more = messages.len() > limit as usize;
        if has_more {
            messages.truncate(limit as usize);
        }

        let message_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
        let tool_calls = self
            .tool_call_repo
            .find_by_message_ids(&message_ids)
            .await
            .unwrap_or_default();

        let mut te_map: std::collections::HashMap<String, Vec<ToolCallResponse>> =
            std::collections::HashMap::new();
        for te in tool_calls {
            te_map
                .entry(te.message_id.clone())
                .or_default()
                .push(te.into());
        }

        let messages = messages
            .into_iter()
            .map(|msg| {
                let id = msg.id.clone();
                let mut resp: MessageResponse = msg.into();
                if let Some(tes) = te_map.remove(&id) {
                    resp.tool_calls = tes;
                }
                resp
            })
            .collect();

        Ok(PaginatedMessagesResponse { messages, has_more })
    }

    pub async fn create_stream_user_message(
        &self,
        user_id: &str,
        chat_id: &str,
        content: &str,
        attachments: Vec<crate::storage::Attachment>,
    ) -> Result<MessageResponse, AppError> {
        self.get_chat(user_id, chat_id).await?;

        let msg = Message::builder(chat_id, MessageRole::User, content.to_string())
            .attachments(attachments)
            .build();
        self.save_message(msg).await
    }

    pub async fn create_contact_message(
        &self,
        user_id: &str,
        chat_id: &str,
        content: &str,
        contact_id: Option<&str>,
    ) -> Result<MessageResponse, AppError> {
        self.get_chat(user_id, chat_id).await?;

        let mut builder = Message::builder(chat_id, MessageRole::Contact, content.to_string());
        if let Some(cid) = contact_id {
            builder = builder.contact_id(cid);
        }
        self.save_message(builder.build()).await
    }

    pub async fn save_live_call_message(
        &self,
        user_id: &str,
        chat_id: &str,
        content: &str,
        contact_id: Option<&str>,
    ) -> Result<MessageResponse, AppError> {
        self.get_chat(user_id, chat_id).await?;

        let mut builder = Message::builder(chat_id, MessageRole::LiveCall, content.to_string());
        if let Some(cid) = contact_id {
            builder = builder.contact_id(cid);
        }
        self.save_message(builder.build()).await
    }

    async fn save_message(&self, message: Message) -> Result<MessageResponse, AppError> {
        let saved = self.message_repo.create(&message).await?;
        Ok(saved.into())
    }

    pub async fn save_system_event(
        &self,
        chat_id: &str,
        event: MessageEvent,
    ) -> Result<MessageResponse, AppError> {
        let msg = Message::builder(chat_id, MessageRole::System, String::new())
            .event(event)
            .build();
        self.save_message(msg).await
    }

    pub async fn save_system_message(
        &self,
        chat_id: &str,
        content: String,
    ) -> Result<MessageResponse, AppError> {
        let msg = Message::builder(chat_id, MessageRole::System, content).build();
        self.save_message(msg).await
    }

    pub async fn save_agent_message(
        &self,
        chat_id: &str,
        agent_id: &str,
        content: String,
        delivery: Option<crate::chat::message::models::MessageDelivery>,
    ) -> Result<MessageResponse, AppError> {
        let mut builder = Message::builder(chat_id, MessageRole::Agent, content)
            .agent_id(agent_id.to_string());
        if let Some(d) = delivery {
            builder = builder.delivery(d);
        }
        let msg = builder.build();
        self.save_message(msg).await
    }

    pub async fn save_task_lifecycle_message(
        &self,
        chat_id: &str,
        agent_id: &str,
        content: String,
        event: MessageEvent,
        attachments: Vec<crate::storage::Attachment>,
    ) -> Result<MessageResponse, AppError> {
        let msg = Message::builder(chat_id, MessageRole::TaskCompletion, content)
            .agent_id(agent_id.to_string())
            .event(event)
            .attachments(attachments)
            .build();
        self.save_message(msg).await
    }

    // ── Tool call persistence ────────────────────────────────────────

    pub async fn create_executing_agent_message(
        &self,
        chat_id: &str,
        agent_id: &str,
        delivery: Option<crate::chat::message::models::MessageDelivery>,
    ) -> Result<MessageResponse, AppError> {
        let mut builder = Message::builder(chat_id, MessageRole::Agent, String::new())
            .agent_id(agent_id.to_string())
            .status(MessageStatus::Executing);
        if let Some(d) = delivery {
            builder = builder.delivery(d);
        }
        let msg = builder.build();
        self.save_message(msg).await
    }

    pub async fn complete_agent_message(
        &self,
        message_id: &str,
        content: String,
        attachments: Vec<crate::storage::Attachment>,
        reasoning: Option<Reasoning>,
    ) -> Result<MessageResponse, AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;

        message.content = content;
        message.attachments = attachments;
        message.reasoning = reasoning;
        message.status = Some(MessageStatus::Completed);

        let updated = self.message_repo.update(&message).await?;
        // Broadcast the completion so the channel outbound dispatcher and
        // any SSE subscribers see the finished agent message — without this
        // the row sits Completed in the DB but no one delivers it externally
        // and the UI shows a perpetual typing indicator.
        if let Ok(Some(chat)) = self.chat_repo.find_by_id(&updated.chat_id).await {
            self.broadcast_message_entity(
                &updated,
                &chat,
                crate::chat::broadcast::EntityAction::Updated,
            );
        }
        Ok(updated.into())
    }

    pub async fn cancel_agent_message(
        &self,
        message_id: &str,
        content: String,
    ) -> Result<MessageResponse, AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;

        message.content = content;
        message.status = Some(MessageStatus::Cancelled);

        let updated = self.message_repo.update(&message).await?;
        Ok(updated.into())
    }

    pub async fn fail_agent_message(&self, message_id: &str) -> Result<(), AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;

        message.status = Some(MessageStatus::Failed);
        self.message_repo.update(&message).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn begin_tool_call(
        &self,
        id: &str,
        chat_id: &str,
        message_id: &str,
        turn: u32,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
        description: Option<String>,
        turn_text: Option<String>,
    ) -> Result<ToolCall, AppError> {
        let te = ToolCall {
            id: id.to_string(),
            chat_id: chat_id.to_string(),
            message_id: message_id.to_string(),
            turn,
            provider_call_id: tool_call_id.to_string(),
            name: tool_name.to_string(),
            arguments: arguments.clone(),
            result: String::new(),
            success: false,
            duration_ms: 0,
            tool_data: None,
            system_prompt: None,
            description,
            turn_text,
            created_at: chrono::Utc::now(),
        };
        self.tool_call_repo.create(&te).await?;
        Ok(te)
    }

    pub async fn finish_tool_call(
        &self,
        tool_call_id: &str,
        result: String,
        success: bool,
        duration_ms: u64,
        tool_data: Option<MessageTool>,
        system_prompt: Option<String>,
    ) -> Result<(), AppError> {
        let mut te = self
            .tool_call_repo
            .find_by_id(tool_call_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Tool call not found".into()))?;
        te.result = result;
        te.success = success;
        te.duration_ms = duration_ms;
        te.tool_data = tool_data;
        te.system_prompt = system_prompt;
        self.tool_call_repo.update(&te).await?;
        Ok(())
    }

    pub async fn resolve_tool_call(
        &self,
        tool_call_id: &str,
        response: Option<String>,
    ) -> Result<ToolResolveResult, AppError> {
        let mut te = self
            .tool_call_repo
            .find_by_id(tool_call_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Tool call not found".into()))?;

        let tool = te.tool_data.as_ref()
            .ok_or_else(|| AppError::Validation("Tool call has no resolvable tool".into()))?;

        if let Some(current_status) = tool.tool_status() {
            match current_status {
                ToolStatus::Resolved => {
                    let existing = tool.tool_response().unwrap_or_default();
                    let incoming = response.as_deref()
                        .unwrap_or("Human resolved the request.");
                    if existing == incoming {
                        return Ok(ToolResolveResult::AlreadyResolved(
                            MessageResponse::from(
                                self.message_repo.find_by_id(&te.message_id).await?
                                    .ok_or_else(|| AppError::NotFound("Message not found".into()))?
                            ),
                        ));
                    }
                    return Err(AppError::Http {
                        status: 409,
                        message: "Tool call is already resolved with a different response".into(),
                    });
                }
                ToolStatus::Denied => {
                    return Err(AppError::Http {
                        status: 409,
                        message: "Tool call has already been denied".into(),
                    });
                }
                ToolStatus::Pending => {}
            }
        } else {
            return Err(AppError::Validation("Tool call has no resolvable tool".into()));
        }

        let response_text = response
            .unwrap_or_else(|| "Human resolved the request.".to_string());

        match &mut te.tool_data {
            Some(MessageTool::HumanInTheLoop { status, response: resp, .. })
            | Some(MessageTool::Question { status, response: resp, .. })
            | Some(MessageTool::VaultApproval { status, response: resp, .. })
            | Some(MessageTool::ServiceApproval { status, response: resp, .. }) => {
                *status = ToolStatus::Resolved;
                *resp = Some(response_text.clone());
            }
            _ => unreachable!(),
        }

        te.result = response_text;
        self.tool_call_repo.update(&te).await?;

        let message = self.message_repo.find_by_id(&te.message_id).await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        Ok(ToolResolveResult::Changed(message.into()))
    }

    pub async fn deny_tool_call(
        &self,
        tool_call_id: &str,
        response: Option<String>,
    ) -> Result<ToolResolveResult, AppError> {
        let mut te = self
            .tool_call_repo
            .find_by_id(tool_call_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Tool call not found".into()))?;

        let tool = te.tool_data.as_ref()
            .ok_or_else(|| AppError::Validation("Tool call has no deniable tool".into()))?;

        if let Some(current_status) = tool.tool_status() {
            match current_status {
                ToolStatus::Denied => {
                    let message = self.message_repo.find_by_id(&te.message_id).await?
                        .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
                    return Ok(ToolResolveResult::AlreadyResolved(message.into()));
                }
                ToolStatus::Resolved => {
                    return Err(AppError::Http {
                        status: 409,
                        message: "Tool call has already been resolved".into(),
                    });
                }
                ToolStatus::Pending => {}
            }
        } else {
            return Err(AppError::Validation("Tool call has no deniable tool".into()));
        }

        let response_text = response
            .unwrap_or_else(|| "User denied the request.".to_string());

        match &mut te.tool_data {
            Some(MessageTool::VaultApproval { status, response: resp, .. })
            | Some(MessageTool::ServiceApproval { status, response: resp, .. })
            | Some(MessageTool::Question { status, response: resp, .. })
            | Some(MessageTool::HumanInTheLoop { status, response: resp, .. }) => {
                *status = ToolStatus::Denied;
                *resp = Some(response_text.clone());
            }
            _ => unreachable!(),
        }

        te.result = response_text;
        self.tool_call_repo.update(&te).await?;

        let message = self.message_repo.find_by_id(&te.message_id).await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        Ok(ToolResolveResult::Changed(message.into()))
    }

    pub async fn find_pending_tool_call(
        &self,
        chat_id: &str,
    ) -> Result<Option<ToolCall>, AppError> {
        self.tool_call_repo.find_pending_by_chat_id(chat_id).await
    }

    pub async fn get_tool_call(
        &self,
        id: &str,
    ) -> Result<Option<ToolCall>, AppError> {
        self.tool_call_repo.find_by_id(id).await
    }

    pub async fn get_tool_calls(
        &self,
        chat_id: &str,
    ) -> Result<Vec<ToolCall>, AppError> {
        self.tool_call_repo.find_by_chat_id(chat_id).await
    }

    pub async fn get_tool_calls_by_message(
        &self,
        message_id: &str,
    ) -> Result<Vec<ToolCall>, AppError> {
        self.tool_call_repo.find_by_message_id(message_id).await
    }

    pub async fn has_pending_tools_for_message(
        &self,
        message_id: &str,
    ) -> Result<bool, AppError> {
        let tes = self.get_tool_calls_by_message(message_id).await?;
        Ok(tes.iter().any(|te| {
            te.tool_data
                .as_ref()
                .and_then(|td| td.tool_status())
                .is_some_and(|s| *s == ToolStatus::Pending)
        }))
    }

    pub async fn find_executing_message_for_chat(
        &self,
        chat_id: &str,
    ) -> Result<Option<Message>, AppError> {
        let query = "SELECT *, meta::id(id) as id FROM message WHERE chat_id = $chat_id AND status = $status LIMIT 1";
        let mut result = self
            .message_repo
            .db()
            .query(query)
            .bind(("chat_id", chat_id.to_string()))
            .bind(("status", MessageStatus::Executing))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let message: Option<Message> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(message)
    }

    pub async fn find_executing_chat_messages(&self) -> Vec<Message> {
        let query = "SELECT *, meta::id(id) as id FROM message WHERE status = $status AND chat_id IN (SELECT VALUE meta::id(id) FROM chat WHERE task_id IS NONE)";
        let result = self
            .message_repo
            .db()
            .query(query)
            .bind(("status", MessageStatus::Executing))
            .await;

        match result {
            Ok(mut r) => {
                let messages: Vec<Message> = r.take(0).unwrap_or_default();
                messages
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to find executing chat messages");
                vec![]
            }
        }
    }

    pub async fn find_message(&self, message_id: &str) -> Result<Option<Message>, AppError> {
        self.message_repo.find_by_id(message_id).await
    }

    pub async fn find_chat(&self, chat_id: &str) -> Result<Option<Chat>, AppError> {
        self.chat_repo.find_by_id(chat_id).await
    }

    pub async fn resolve_agent_config(&self, agent_id: &str) -> Result<AgentConfig, AppError> {
        let ws = self.storage_service.agent_workspace(agent_id);

        if let Ok(Some(agent)) = self.agent_service.find_by_id(agent_id).await {
            tracing::debug!(agent_id, user_id = ?agent.user_id, "Resolved agent from DB");

            let raw_prompt = if let Some(ref prompt) = agent.prompt {
                if !prompt.is_empty() {
                    prompt.clone()
                } else {
                    ws.read("AGENT.md")
                        .map(|c| parse_frontmatter(&c).template)
                        .ok_or_else(|| AppError::Internal(format!("No AGENT.md found for agent {agent_id}")))?
                }
            } else {
                ws.read("AGENT.md")
                    .map(|c| parse_frontmatter(&c).template)
                    .ok_or_else(|| AppError::Internal(format!("No AGENT.md found for agent {agent_id}")))?
            };

            let system_prompt = render_template(&raw_prompt, &[("agent_name", &agent.name)])
                .unwrap_or(raw_prompt);

            return Ok(AgentConfig {
                system_prompt,
                model_group: agent.model_group,
                skills: agent.skills,
                sandbox_limits: agent.sandbox_limits,
                identity: agent.identity,
            });
        }

        let raw_content = ws.read("AGENT.md")
            .ok_or_else(|| AppError::Internal(format!("No AGENT.md found for agent {agent_id}")))?;
        let parsed = parse_frontmatter(&raw_content);

        let model_group = parsed.metadata.get("model_group")
            .cloned()
            .unwrap_or_else(|| "primary".to_string());

        Ok(AgentConfig {
            system_prompt: parsed.template,
            model_group,
            skills: None,
            sandbox_limits: None,
            identity: std::collections::BTreeMap::new(),
        })
    }

    pub async fn generate_title(
        &self,
        chat_id: &str,
        agent_id: &str,
        user_content: &str,
    ) -> Result<String, AppError> {
        let ws = self.storage_service.agent_workspace(agent_id);
        let prompts = AgentPromptLoader::new(&ws, &self.prompts);
        let content = prompts.read("TITLE.md")
            .ok_or_else(|| AppError::Internal("No title generation prompt found".into()))?;
        let parsed = parse_frontmatter(&content);

        let model_group = self.build_title_model_group(parsed.metadata.get("model").map(|s| s.as_str()))?;

        let result = text_inference(
            &self.provider_registry,
            &model_group,
            &parsed.template,
            vec![RigMessage::user(user_content)],
            &InferenceMetricsContext::default(),
        )
        .await?;

        let title = parse_title_response(&result, user_content);
        self.update_chat_title(chat_id, &title).await?;
        Ok(title)
    }

    fn build_title_model_group(&self, model_specifier: Option<&str>) -> Result<crate::inference::config::ModelGroup, AppError> {
        let base = match model_specifier {
            Some(m) if m.contains('/') => {
                let model_ref = ModelRef::parse(m)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                return Ok(crate::inference::config::ModelGroup {
                    name: "title".to_string(),
                    main: model_ref,
                    fallbacks: vec![],
                    max_tokens: Some(100),
                    temperature: Some(0.7),
                    context_window: None,
                    retry: Default::default(),
                    inference: Default::default(),
                });
            }
            Some(group) if !group.is_empty() => {
                self.provider_registry.get_model_group(group)?
            }
            _ => self.provider_registry.get_model_group("primary")?,
        };
        Ok(crate::inference::config::ModelGroup {
            name: "title".to_string(),
            main: base.main.clone(),
            fallbacks: base.fallbacks.clone(),
            max_tokens: Some(100),
            temperature: Some(0.7),
            context_window: None,
            retry: base.retry.clone(),
            inference: base.inference.clone(),
        })
    }

    async fn update_chat_title(&self, chat_id: &str, title: &str) -> Result<(), AppError> {
        if let Some(mut chat) = self.chat_repo.find_by_id(chat_id).await? {
            chat.title = Some(title.to_string());
            chat.updated_at = chrono::Utc::now();
            self.chat_repo.update(&chat).await?;
        }
        Ok(())
    }

    pub async fn get_stored_messages(&self, chat_id: &str) -> Result<Vec<Message>, AppError> {
        self.message_repo.find_by_chat_id(chat_id).await
    }

    pub async fn find_chats_by_space_id(&self, space_id: &str) -> Result<Vec<Chat>, AppError> {
        self.chat_repo.find_by_space_id(space_id).await
    }

    pub async fn find_standalone_chats_by_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<Chat>, AppError> {
        self.chat_repo.find_standalone_by_user_id(user_id).await
    }

    pub async fn find_attachments_by_chat_id(
        &self,
        chat_id: &str,
    ) -> Result<Vec<crate::storage::Attachment>, AppError> {
        self.message_repo.find_attachments_by_chat_id(chat_id).await
    }
}

fn parse_title_response(response: &str, fallback_content: &str) -> String {
    if let Some(title) = try_extract_title(response) {
        return title;
    }

    tracing::debug!(response = %response, "Failed to parse title from LLM response, using fallback");
    fallback_content.chars().take(60).collect()
}

fn try_extract_title(response: &str) -> Option<String> {
    let trimmed = response.trim();

    if let Some(title) = try_parse_title_json(trimmed) {
        return Some(title);
    }

    let open = trimmed.find('{')?;
    let close = trimmed.rfind('}')?;
    if open < close
        && let Some(title) = try_parse_title_json(&trimmed[open..=close])
    {
        return Some(title);
    }

    None
}

fn try_parse_title_json(s: &str) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(s).ok()?;
    let title = v.get("title")?.as_str()?;
    if title.is_empty() {
        return None;
    }
    Some(title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_title_response_valid_json() {
        let response = r#"{ "title": "🍝 Pasta Carbonara Recipe" }"#;
        assert_eq!(
            parse_title_response(response, "fallback"),
            "🍝 Pasta Carbonara Recipe"
        );
    }

    #[test]
    fn test_parse_title_response_markdown_fenced() {
        let response = "```json\n{ \"title\": \"🍝 Pasta\" }\n```";
        assert_eq!(parse_title_response(response, "fallback"), "🍝 Pasta");
    }

    #[test]
    fn test_parse_title_response_markdown_fenced_multiline() {
        let response = "```json\n{\n  \"title\": \"⚽ World Cup Organizer\"\n}\n```";
        assert_eq!(
            parse_title_response(response, "fallback"),
            "⚽ World Cup Organizer"
        );
    }

    #[test]
    fn test_parse_title_response_extra_text_around_json() {
        let response = "Here is the title:\n{ \"title\": \"🎯 Test\" }\nDone.";
        assert_eq!(parse_title_response(response, "fallback"), "🎯 Test");
    }

    #[test]
    fn test_parse_title_response_invalid_json() {
        let response = "not json at all";
        assert_eq!(
            parse_title_response(response, "How do I make pasta carbonara?"),
            "How do I make pasta carbonara?"
        );
    }

    #[test]
    fn test_parse_title_response_empty_title() {
        let response = r#"{ "title": "" }"#;
        assert_eq!(parse_title_response(response, "fallback text"), "fallback text");
    }
}
