use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::error::AppError;
use crate::memory::basic::BasicMemoryService;
use frona_derive::agent_tool;

use crate::tool::{InferenceContext, ToolOutput, active_chat, str_list_arg};

pub struct StoreAgentMemoryTool {
    memory_service: BasicMemoryService,
    prompts: PromptLoader,
}

impl StoreAgentMemoryTool {
    pub fn new(memory_service: BasicMemoryService, prompts: PromptLoader) -> Self {
        Self {
            memory_service,
            prompts,
        }
    }
}

#[agent_tool(name = "store_agent_memory")]
impl StoreAgentMemoryTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        // Storing is per-fact, calling needn't be: a turn that learned three things
        // about the user cost three tool turns, and each spawned its own compaction.
        let memories = str_list_arg(&arguments, "memory", "memories");
        if memories.is_empty() {
            return Err(AppError::Validation("Missing 'memory' parameter".into()));
        }

        let overrides = arguments
            .get("overrides")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let agent_id = &ctx.agent.id;
        let chat = active_chat(ctx)?;
        let chat_id = &chat.id;

        tracing::debug!(
            agent_id = %agent_id,
            memories = ?memories,
            overrides = overrides,
            "store_agent_memory tool called"
        );

        for memory in &memories {
            self.memory_service
                .store_memory_entry(agent_id, memory, Some(chat_id))
                .await?;
        }

        match self
            .memory_service
            .compaction_model_group(&ctx.agent.model_group)
        {
            Ok(group) => {
                let ms = self.memory_service.clone();
                let aid = agent_id.clone();
                let uid = ctx.user.id.clone();
                if overrides {
                    tracing::debug!(agent_id = %aid, "Spawning forced memory compaction (overrides=true)");
                    tokio::spawn(async move {
                        if let Err(e) = ms.compact_entries_forced(&uid, &aid, &group).await {
                            tracing::warn!(error = %e, agent_id = %aid, "Background forced memory compaction failed");
                        }
                    });
                } else {
                    tracing::debug!(agent_id = %aid, "Spawning background memory compaction");
                    tokio::spawn(async move {
                        if let Err(e) = ms.compact_entries_if_needed(&uid, &aid, &group).await {
                            tracing::warn!(error = %e, agent_id = %aid, "Background memory compaction failed");
                        }
                    });
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "Stored memory without compaction: no model group available");
            }
        }

        Ok(ToolOutput::text(format!(
            "Stored: {}",
            memories.join(" | ")
        )))
    }
}

pub struct StoreUserMemoryTool {
    memory_service: BasicMemoryService,
    prompts: PromptLoader,
}

impl StoreUserMemoryTool {
    pub fn new(memory_service: BasicMemoryService, prompts: PromptLoader) -> Self {
        Self {
            memory_service,
            prompts,
        }
    }
}

#[agent_tool]
impl StoreUserMemoryTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let memories = str_list_arg(&arguments, "memory", "memories");
        if memories.is_empty() {
            return Err(AppError::Validation("Missing 'memory' parameter".into()));
        }

        let overrides = arguments
            .get("overrides")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let user_id = &ctx.user.id;
        let chat = active_chat(ctx)?;
        let chat_id = &chat.id;

        tracing::debug!(
            user_id = %user_id,
            memories = ?memories,
            overrides = overrides,
            "store_user_memory tool called"
        );

        for memory in &memories {
            self.memory_service
                .store_user_memory_entry(user_id, memory, Some(chat_id))
                .await?;
        }

        match self
            .memory_service
            .compaction_model_group(&ctx.agent.model_group)
        {
            Ok(group) => {
                let ms = self.memory_service.clone();
                let uid = user_id.clone();
                if overrides {
                    tracing::debug!(user_id = %uid, "Spawning forced user memory compaction (overrides=true)");
                    tokio::spawn(async move {
                        if let Err(e) = ms.compact_user_entries_forced(&uid, &group).await {
                            tracing::warn!(error = %e, user_id = %uid, "Background forced user memory compaction failed");
                        }
                    });
                } else {
                    tracing::debug!(user_id = %uid, "Spawning background user memory compaction");
                    tokio::spawn(async move {
                        if let Err(e) = ms.compact_user_entries_if_needed(&uid, &group).await {
                            tracing::warn!(error = %e, user_id = %uid, "Background user memory compaction failed");
                        }
                    });
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "Stored memory without compaction: no model group available");
            }
        }

        Ok(ToolOutput::text(format!(
            "Stored for user: {}",
            memories.join(" | ")
        )))
    }
}
