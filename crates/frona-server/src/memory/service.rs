use std::sync::Arc;

use chrono::{DateTime, Utc};
use rig_core::completion::Message as RigMessage;

use crate::agent::prompt::append_tagged_section;
use crate::agent::workspace::AgentPromptLoader;
use crate::storage::StorageService;
use crate::db::repo::memory_entries::SurrealMemoryEntryRepo;
use crate::db::repo::memories::SurrealMemoryRepo;
use crate::db::repo::messages::SurrealMessageRepo;
use crate::chat::message::models::Message;
use crate::chat::message::repository::MessageRepository;
use crate::core::error::AppError;
use crate::inference::config::ModelGroup;
use crate::inference::context::estimate_tokens;
use crate::inference::conversation::{
    convert_agent_message, format_files_block_simple,
};
use crate::inference::text_inference;
use crate::inference::ModelProviderRegistry;
use crate::memory::models::{Memory, MemoryEntry, MemorySourceType};
use crate::memory::repository::{MemoryRepository, MemoryEntryRepository};
use crate::agent::prompt::PromptLoader;
use crate::core::repository::Repository;

const MEMORY_COMPACTION_TOKEN_THRESHOLD: usize = 3_000;

#[derive(Clone)]
pub struct MemoryService {
    memory_repo: SurrealMemoryRepo,
    memory_entry_repo: SurrealMemoryEntryRepo,
    message_repo: SurrealMessageRepo,
    provider_registry: Arc<ModelProviderRegistry>,
    prompts: PromptLoader,
    storage: StorageService,
    usage_service: crate::inference::usage::UsageService,
}

impl MemoryService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory_repo: SurrealMemoryRepo,
        memory_entry_repo: SurrealMemoryEntryRepo,
        message_repo: SurrealMessageRepo,
        provider_registry: Arc<ModelProviderRegistry>,
        prompts: PromptLoader,
        storage: StorageService,
        usage_service: crate::inference::usage::UsageService,
    ) -> Self {
        Self {
            memory_repo,
            memory_entry_repo,
            message_repo,
            provider_registry,
            prompts,
            storage,
            usage_service,
        }
    }

    fn load_prompt(&self, name: &str, agent: Option<(&crate::core::Handle, &crate::core::Handle)>) -> Option<String> {
        if let Some((user_handle, agent_handle)) = agent {
            let ws = self.storage.agent_workspace(user_handle, agent_handle);
            let loader = AgentPromptLoader::new(&ws, &self.prompts);
            return loader.read(name);
        }
        self.prompts.read(name)
    }

    /// Forced wrapper for `/compact` — looks up the compaction model group
    /// from the registry (falling back to `primary` if `compaction` isn't
    /// configured) and threads through to `compact_chat_if_needed`. The
    /// threshold check still applies — under-threshold chats are a no-op.
    /// Returns a short status string suitable for an assistant message.
    pub async fn compact_chat_via_command(
        &self,
        user_id: &str,
        chat_id: &str,
        chat_agent_id: &str,
        system_prompt: &str,
        context_window: usize,
        max_output_tokens: usize,
    ) -> Result<&'static str, AppError> {
        let compaction_group = self
            .provider_registry
            .get_model_group("compaction")
            .or_else(|_| self.provider_registry.get_model_group("primary"))
            .map_err(|e| AppError::Internal(format!("No compaction model group available: {e}")))?;

        let before = self
            .memory_repo
            .find_latest(MemorySourceType::Chat, chat_id)
            .await?
            .map(|m| m.updated_at);

        self.compact_chat_if_needed(
            user_id,
            chat_id,
            chat_agent_id,
            system_prompt,
            context_window,
            max_output_tokens,
            compaction_group,
        )
        .await?;

        let after = self
            .memory_repo
            .find_latest(MemorySourceType::Chat, chat_id)
            .await?
            .map(|m| m.updated_at);

        if before == after {
            Ok("Chat is already at optimal size — no compaction needed.")
        } else {
            Ok("Compacted older messages into a summary.")
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn compact_chat_if_needed(
        &self,
        user_id: &str,
        chat_id: &str,
        chat_agent_id: &str,
        system_prompt: &str,
        context_window: usize,
        max_output_tokens: usize,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        let messages = self.message_repo.find_by_chat_id(chat_id).await?;
        if messages.is_empty() {
            return Ok(());
        }

        let rig_messages: Vec<RigMessage> = messages
            .iter()
            .filter_map(|msg| match msg.role {
                crate::chat::message::models::MessageRole::User
                | crate::chat::message::models::MessageRole::TaskCompletion
                | crate::chat::message::models::MessageRole::Contact => {
                    let content = format_files_block_simple(&msg.content, &msg.attachments);
                    Some(RigMessage::user(&content))
                }
                crate::chat::message::models::MessageRole::LiveCall => {
                    let content = format_files_block_simple(&msg.content, &msg.attachments);
                    Some(RigMessage::user(format!("[LIVE_CALL] {content}")))
                }
                crate::chat::message::models::MessageRole::Agent => {
                    convert_agent_message(msg, chat_agent_id, None)
                }
                crate::chat::message::models::MessageRole::System => None,
            })
            .collect();
        let available = context_window.saturating_sub(max_output_tokens);

        let mut total_tokens = estimate_tokens(system_prompt);
        for msg in &rig_messages {
            total_tokens += crate::inference::context::estimate_message_tokens(msg);
        }

        if total_tokens <= available * 80 / 100 {
            return Ok(());
        }

        let existing_memory = self
            .memory_repo
            .find_latest(MemorySourceType::Chat, chat_id)
            .await?;

        let target = available * 70 / 100;
        let mut summary_budget = estimate_tokens(system_prompt);
        if let Some(ref mem) = existing_memory {
            summary_budget += estimate_tokens(&mem.content);
        }

        let mut keep_from_idx = messages.len();
        let mut running = 0usize;
        for (i, msg) in rig_messages.iter().enumerate().rev() {
            let cost = crate::inference::context::estimate_message_tokens(msg);
            if running + cost + summary_budget > target {
                break;
            }
            running += cost;
            keep_from_idx = i;
        }

        if keep_from_idx == 0 {
            return Ok(());
        }

        let messages_to_compact = &messages[..keep_from_idx];

        let mut compaction_input = String::new();
        if let Some(ref mem) = existing_memory {
            compaction_input.push_str("Previous summary:\n");
            compaction_input.push_str(&mem.content);
            compaction_input.push_str("\n\nNew messages to incorporate:\n");
        }
        for msg in messages_to_compact {
            let role_str = match msg.role {
                crate::chat::message::models::MessageRole::User => "User",
                crate::chat::message::models::MessageRole::Agent => "Agent",
                crate::chat::message::models::MessageRole::TaskCompletion => "System",
                crate::chat::message::models::MessageRole::Contact => "Contact",
                crate::chat::message::models::MessageRole::LiveCall => "Caller",
                crate::chat::message::models::MessageRole::System => continue,
            };
            compaction_input.push_str(&format!("{role_str}: {}\n", msg.content));
        }

        let prompt = self.load_prompt("CHAT_COMPACTION.md", None)
            .expect("built-in CHAT_COMPACTION.md missing");
        let usage_ctx = crate::inference::usage::UsageContext::new(
            crate::inference::usage::InferenceKind::Compaction {
                target: crate::inference::usage::CompactionTarget::Chat {
                    agent_id: chat_agent_id.to_string(),
                    chat_id: chat_id.to_string(),
                },
            },
            user_id,
            compaction_model_group.name.clone(),
        );
        let summary = text_inference(
            &self.provider_registry,
            compaction_model_group,
            &prompt,
            vec![RigMessage::user(&compaction_input)],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        .map_err(|e| AppError::Internal(format!("Chat compaction failed: {e}")))?;

        let now = Utc::now();
        let compacted_until = messages_to_compact
            .last()
            .map(|m| m.created_at)
            .unwrap_or(now);

        let memory = Memory {
            id: existing_memory
                .as_ref()
                .map(|m| m.id.clone())
                .unwrap_or_else(crate::core::repository::new_id),
            source_type: MemorySourceType::Chat,
            source_id: chat_id.to_string(),
            content: summary,
            metadata: serde_json::json!({
                "compacted_until": compacted_until,
                "item_count": messages_to_compact.len(),
            }),
            created_at: existing_memory
                .as_ref()
                .map(|m| m.created_at)
                .unwrap_or(now),
            updated_at: now,
        };

        if existing_memory.is_some() {
            self.memory_repo.update(&memory).await?;
        } else {
            self.memory_repo.create(&memory).await?;
        }

        for msg in messages_to_compact {
            self.message_repo.delete(&msg.id).await?;
        }

        Ok(())
    }

    pub async fn store_memory_entry(
        &self,
        agent_id: &str,
        content: &str,
        source_chat_id: Option<&str>,
    ) -> Result<MemoryEntry, AppError> {
        tracing::debug!(agent_id = %agent_id, content = %content, "Storing agent memory entry");

        let entry = MemoryEntry {
            id: crate::core::repository::new_id(),
            agent_id: agent_id.to_string(),
            user_id: None,
            content: content.to_string(),
            source_chat_id: source_chat_id.map(|s| s.to_string()),
            created_at: Utc::now(),
        };

        self.memory_entry_repo.create(&entry).await
    }

    pub async fn store_user_memory_entry(
        &self,
        user_id: &str,
        content: &str,
        source_chat_id: Option<&str>,
    ) -> Result<MemoryEntry, AppError> {
        tracing::debug!(user_id = %user_id, content = %content, "Storing user memory entry");

        let entry = MemoryEntry {
            id: crate::core::repository::new_id(),
            agent_id: String::new(),
            user_id: Some(user_id.to_string()),
            content: content.to_string(),
            source_chat_id: source_chat_id.map(|s| s.to_string()),
            created_at: Utc::now(),
        };

        self.memory_entry_repo.create(&entry).await
    }

    pub async fn compact_entries_if_needed(
        &self,
        user_id: &str,
        agent_id: &str,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        let entries = self.memory_entry_repo.find_by_agent_id(agent_id).await?;
        let total_tokens: usize = entries.iter().map(|e| estimate_tokens(&e.content)).sum();

        if total_tokens <= MEMORY_COMPACTION_TOKEN_THRESHOLD {
            tracing::debug!(
                agent_id = %agent_id,
                token_count = total_tokens,
                threshold = MEMORY_COMPACTION_TOKEN_THRESHOLD,
                "Skipping memory compaction (below threshold)"
            );
            return Ok(());
        }

        self.compact_entries(user_id, agent_id, MemorySourceType::Agent, entries, compaction_model_group)
            .await
    }

    pub async fn compact_entries_forced(
        &self,
        user_id: &str,
        agent_id: &str,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        let entries = self.memory_entry_repo.find_by_agent_id(agent_id).await?;
        if entries.is_empty() {
            return Ok(());
        }
        self.compact_entries(user_id, agent_id, MemorySourceType::Agent, entries, compaction_model_group)
            .await
    }

    pub async fn compact_user_entries_if_needed(
        &self,
        user_id: &str,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        let entries = self.memory_entry_repo.find_by_user_id(user_id).await?;
        let total_tokens: usize = entries.iter().map(|e| estimate_tokens(&e.content)).sum();

        if total_tokens <= MEMORY_COMPACTION_TOKEN_THRESHOLD {
            tracing::debug!(
                user_id = %user_id,
                token_count = total_tokens,
                threshold = MEMORY_COMPACTION_TOKEN_THRESHOLD,
                "Skipping user memory compaction (below threshold)"
            );
            return Ok(());
        }

        self.compact_user_entries(user_id, entries, compaction_model_group)
            .await
    }

    pub async fn compact_user_entries_forced(
        &self,
        user_id: &str,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        let entries = self.memory_entry_repo.find_by_user_id(user_id).await?;
        if entries.is_empty() {
            return Ok(());
        }
        self.compact_user_entries(user_id, entries, compaction_model_group)
            .await
    }

    async fn compact_user_entries(
        &self,
        user_id: &str,
        entries: Vec<MemoryEntry>,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        let token_count_before: usize = entries.iter().map(|e| estimate_tokens(&e.content)).sum();
        tracing::info!(
            user_id = %user_id,
            entry_count = entries.len(),
            token_count = token_count_before,
            "Running user memory compaction"
        );

        let existing_memory = self
            .memory_repo
            .find_latest(MemorySourceType::User, user_id)
            .await?;

        let mut compaction_input = String::new();
        if let Some(ref mem) = existing_memory {
            compaction_input.push_str("Previous user memory:\n");
            compaction_input.push_str(&mem.content);
            compaction_input.push_str("\n\nNew memories to incorporate:\n");
        }
        for entry in &entries {
            compaction_input.push_str(&format!("- {}\n", entry.content));
        }

        let prompt = self.load_prompt("MEMORY_COMPACTION.md", None)
            .expect("built-in MEMORY_COMPACTION.md missing");
        let usage_ctx = crate::inference::usage::UsageContext::new(
            crate::inference::usage::InferenceKind::Compaction {
                target: crate::inference::usage::CompactionTarget::User,
            },
            user_id,
            compaction_model_group.name.clone(),
        );
        let summary = text_inference(
            &self.provider_registry,
            compaction_model_group,
            &prompt,
            vec![RigMessage::user(&compaction_input)],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        .map_err(|e| AppError::Internal(format!("User memory compaction failed: {e}")))?;

        let token_count_after = estimate_tokens(&summary);
        tracing::info!(
            user_id = %user_id,
            token_count_before,
            token_count_after,
            "User memory compaction complete"
        );

        let now = Utc::now();
        let last_entry_time = entries.last().map(|e| e.created_at).unwrap_or(now);

        let memory = Memory {
            id: existing_memory
                .as_ref()
                .map(|m| m.id.clone())
                .unwrap_or_else(crate::core::repository::new_id),
            source_type: MemorySourceType::User,
            source_id: user_id.to_string(),
            content: summary,
            metadata: serde_json::json!({
                "compacted_until": last_entry_time,
                "item_count": entries.len(),
            }),
            created_at: existing_memory
                .as_ref()
                .map(|m| m.created_at)
                .unwrap_or(now),
            updated_at: now,
        };

        if existing_memory.is_some() {
            self.memory_repo.update(&memory).await?;
        } else {
            self.memory_repo.create(&memory).await?;
        }

        self.memory_entry_repo
            .delete_by_user_id_before(user_id, last_entry_time)
            .await?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn compact_entries(
        &self,
        user_id: &str,
        source_id: &str,
        source_type: MemorySourceType,
        entries: Vec<MemoryEntry>,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        let token_count_before: usize = entries.iter().map(|e| estimate_tokens(&e.content)).sum();
        tracing::info!(
            source_id = %source_id,
            entry_count = entries.len(),
            token_count = token_count_before,
            "Running memory compaction"
        );

        let existing_memory = self
            .memory_repo
            .find_latest(source_type.clone(), source_id)
            .await?;

        let mut compaction_input = String::new();
        if let Some(ref mem) = existing_memory {
            compaction_input.push_str("Previous agent memory:\n");
            compaction_input.push_str(&mem.content);
            compaction_input.push_str("\n\nNew memories to incorporate:\n");
        }
        for entry in &entries {
            compaction_input.push_str(&format!("- {}\n", entry.content));
        }

        let prompt = self.load_prompt("MEMORY_COMPACTION.md", None)
            .expect("built-in MEMORY_COMPACTION.md missing");
        let target = match source_type {
            MemorySourceType::Agent => crate::inference::usage::CompactionTarget::Agent {
                agent_id: source_id.to_string(),
            },
            MemorySourceType::Space => crate::inference::usage::CompactionTarget::Space {
                space_id: source_id.to_string(),
            },
            _ => crate::inference::usage::CompactionTarget::User,
        };
        let usage_ctx = crate::inference::usage::UsageContext::new(
            crate::inference::usage::InferenceKind::Compaction { target },
            user_id,
            compaction_model_group.name.clone(),
        );
        let summary = text_inference(
            &self.provider_registry,
            compaction_model_group,
            &prompt,
            vec![RigMessage::user(&compaction_input)],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        .map_err(|e| AppError::Internal(format!("Memory compaction failed: {e}")))?;

        let token_count_after = estimate_tokens(&summary);
        tracing::info!(
            source_id = %source_id,
            token_count_before,
            token_count_after,
            "Memory compaction complete"
        );

        let now = Utc::now();
        let last_entry_time = entries.last().map(|e| e.created_at).unwrap_or(now);

        let memory = Memory {
            id: existing_memory
                .as_ref()
                .map(|m| m.id.clone())
                .unwrap_or_else(crate::core::repository::new_id),
            source_type,
            source_id: source_id.to_string(),
            content: summary,
            metadata: serde_json::json!({
                "compacted_until": last_entry_time,
                "item_count": entries.len(),
            }),
            created_at: existing_memory
                .as_ref()
                .map(|m| m.created_at)
                .unwrap_or(now),
            updated_at: now,
        };

        if existing_memory.is_some() {
            self.memory_repo.update(&memory).await?;
        } else {
            self.memory_repo.create(&memory).await?;
        }

        self.memory_entry_repo
            .delete_by_agent_id_before(source_id, last_entry_time)
            .await?;

        Ok(())
    }

    pub async fn compact_space(
        &self,
        user_id: &str,
        space_id: &str,
        chat_summaries: Vec<(String, String)>,
        compaction_model_group: &ModelGroup,
    ) -> Result<(), AppError> {
        if chat_summaries.is_empty() {
            return Ok(());
        }

        let mut input = String::new();
        for (title, summary) in &chat_summaries {
            input.push_str(&format!("## {title}\n{summary}\n\n"));
        }

        let prompt = self.load_prompt("SPACE_COMPACTION.md", None)
            .expect("built-in SPACE_COMPACTION.md missing");
        let usage_ctx = crate::inference::usage::UsageContext::new(
            crate::inference::usage::InferenceKind::Compaction {
                target: crate::inference::usage::CompactionTarget::Space {
                    space_id: space_id.to_string(),
                },
            },
            user_id,
            compaction_model_group.name.clone(),
        );
        let summary = text_inference(
            &self.provider_registry,
            compaction_model_group,
            &prompt,
            vec![RigMessage::user(&input)],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        .map_err(|e| AppError::Internal(format!("Space compaction failed: {e}")))?;

        let now = Utc::now();
        let existing_memory = self
            .memory_repo
            .find_latest(MemorySourceType::Space, space_id)
            .await?;

        let memory = Memory {
            id: existing_memory
                .as_ref()
                .map(|m| m.id.clone())
                .unwrap_or_else(crate::core::repository::new_id),
            source_type: MemorySourceType::Space,
            source_id: space_id.to_string(),
            content: summary,
            metadata: serde_json::json!({
                "chat_count": chat_summaries.len(),
            }),
            created_at: existing_memory
                .as_ref()
                .map(|m| m.created_at)
                .unwrap_or(now),
            updated_at: now,
        };

        if existing_memory.is_some() {
            self.memory_repo.update(&memory).await?;
        } else {
            self.memory_repo.create(&memory).await?;
        }

        Ok(())
    }

    pub async fn get_memory(
        &self,
        source_type: MemorySourceType,
        source_id: &str,
    ) -> Result<Option<Memory>, AppError> {
        self.memory_repo.find_latest(source_type, source_id).await
    }

    pub async fn get_conversation_context(
        &self,
        chat_id: &str,
    ) -> Result<(Option<String>, Vec<Message>), AppError> {
        let memory = self
            .memory_repo
            .find_latest(MemorySourceType::Chat, chat_id)
            .await?;

        match memory {
            Some(mem) => {
                let compacted_until: Option<DateTime<Utc>> = mem
                    .metadata
                    .get("compacted_until")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok());

                let messages = match compacted_until {
                    Some(until) => {
                        self.message_repo
                            .find_by_chat_id(chat_id)
                            .await?
                            .into_iter()
                            .filter(|m| m.created_at > until)
                            .collect()
                    }
                    None => self.message_repo.find_by_chat_id(chat_id).await?,
                };

                Ok((Some(mem.content), messages))
            }
            None => {
                let messages = self.message_repo.find_by_chat_id(chat_id).await?;
                Ok((None, messages))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build_augmented_system_prompt(
        &self,
        base_prompt: &str,
        agent_id: &str,
        agent_handle: &crate::core::Handle,
        user_id: &str,
        user_handle: &crate::core::Handle,
        space_id: Option<&str>,
        skills: &[crate::agent::skill::resolver::Skill],
        agent_summaries: &[(String, String)],
        identity: &std::collections::BTreeMap<String, String>,
        mcp_servers: &[(String, String)],
        user_timezone: &str,
    ) -> Result<String, AppError> {
        // Prompt is ordered static → almost-static → dynamic to maximise
        // the cacheable prefix for LLM prompt caching.

        let mut result = base_prompt.to_string();

        const CORE_IDENTITY_KEYS: &[&str] = &["name", "creature", "vibe"];
        let has_core_identity = CORE_IDENTITY_KEYS
            .iter()
            .all(|core_key| identity.keys().any(|k| k.eq_ignore_ascii_case(core_key)));

        if !has_core_identity
            && let Some(identity_prompt) = self.load_prompt("IDENTITY.md", Some((user_handle, agent_handle)))
        {
            result.push_str("\n\n");
            result.push_str(&identity_prompt);
        }

        const AGENT_PROMPTS: &[&str] = &["WORKSPACE.md", "TOOLS.md", "SKILLS.md", "MEMORY.md", "SCHEDULING.md"];
        for name in AGENT_PROMPTS {
            if let Some(content) = self.prompts.read(name) {
                result.push_str("\n\n");
                result.push_str(&content);
            }
        }

        // Skills flagged `disable-model-invocation: true` stay user-invocable
        // (visible in the `/` dropdown) but are hidden from the model's
        // `<available_skills>` block so it can't auto-trigger them.
        let skill_items: Vec<(String, String)> = skills
            .iter()
            .filter(|s| !s.disable_model_invocation)
            .map(|s| (s.name.clone(), format!("{} (file: {}/SKILL.md)", s.description, s.path)))
            .collect();
        append_tagged_section(
            &mut result,
            "available_skills",
            None,
            &skill_items,
        );

        if !mcp_servers.is_empty() {
            if let Some(mcp_prompt) = self.prompts.read("MCP.md") {
                result.push_str("\n\n");
                result.push_str(&mcp_prompt);
            }
            append_tagged_section(
                &mut result,
                "mcpservers",
                None,
                mcp_servers,
            );
        }

        append_tagged_section(
            &mut result,
            "available_agents",
            self.prompts.read("AVAILABLE_AGENTS.md").as_deref(),
            agent_summaries,
        );

        let identity_pairs: Vec<(String, String)> =
            identity.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        append_tagged_section(
            &mut result,
            "agent_identity",
            None,
            &identity_pairs,
        );

        if let Some(sid) = space_id
            && let Some(space_mem) = self
                .get_memory(MemorySourceType::Space, sid)
                .await?
        {
            result.push_str("\n\n<space_context>\n");
            result.push_str(&space_mem.content);
            result.push_str("\n</space_context>");
        }

        if let Some(user_mem) = self
            .get_memory(MemorySourceType::User, user_id)
            .await?
        {
            tracing::debug!(
                user_id = %user_id,
                memory_len = user_mem.content.len(),
                "Using compacted user memory"
            );
            result.push_str("\n\n<user_memory>\n");
            result.push_str(&user_mem.content);

            let compacted_until = user_mem
                .metadata
                .get("compacted_until")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok());

            let new_entries = match compacted_until {
                Some(until) => {
                    self.memory_entry_repo
                        .find_by_user_id_after(user_id, until)
                        .await?
                }
                None => self.memory_entry_repo.find_by_user_id(user_id).await?,
            };
            if !new_entries.is_empty() {
                result.push('\n');
                for entry in &new_entries {
                    result.push_str(&format!("- {}\n", entry.content));
                }
            }

            result.push_str("</user_memory>");
        } else {
            let entries = self.memory_entry_repo.find_by_user_id(user_id).await?;
            if !entries.is_empty() {
                tracing::debug!(
                    user_id = %user_id,
                    entry_count = entries.len(),
                    "No compacted user memory, using raw entries"
                );
                result.push_str("\n\n<user_memory>\n");
                for entry in &entries {
                    result.push_str(&format!("- {}\n", entry.content));
                }
                result.push_str("</user_memory>");
            }
        }

        if let Some(agent_mem) = self
            .get_memory(MemorySourceType::Agent, agent_id)
            .await?
        {
            tracing::debug!(
                agent_id = %agent_id,
                memory_len = agent_mem.content.len(),
                "Using compacted agent memory"
            );
            result.push_str("\n\n<agent_memory>\n");
            result.push_str(&agent_mem.content);

            let compacted_until = agent_mem
                .metadata
                .get("compacted_until")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok());

            let new_entries = match compacted_until {
                Some(until) => {
                    self.memory_entry_repo
                        .find_by_agent_id_after(agent_id, until)
                        .await?
                }
                None => self.memory_entry_repo.find_by_agent_id(agent_id).await?,
            };
            if !new_entries.is_empty() {
                result.push('\n');
                for entry in &new_entries {
                    result.push_str(&format!("- {}\n", entry.content));
                }
            }

            result.push_str("</agent_memory>");
        } else {
            let entries = self.memory_entry_repo.find_by_agent_id(agent_id).await?;
            tracing::debug!(
                agent_id = %agent_id,
                entry_count = entries.len(),
                "No compacted agent memory, using raw entries"
            );
            if !entries.is_empty() {
                result.push_str("\n\n<agent_memory>\n");
                for entry in &entries {
                    result.push_str(&format!("- {}\n", entry.content));
                }
                result.push_str("</agent_memory>");
            }
        }

        // Date-only (no time-of-day) keeps this byte-stable across requests within
        // a day so provider prefix caches stay warm. Don't add hour/minute here.
        let tz: chrono_tz::Tz = user_timezone.parse().unwrap_or(chrono_tz::UTC);
        let now_local = chrono::Utc::now().with_timezone(&tz);
        let items = vec![
            (
                "current_date_local".to_string(),
                format!("{} ({})", now_local.format("%Y-%m-%d"), now_local.format("%A")),
            ),
            ("user_timezone".to_string(), user_timezone.to_string()),
        ];
        append_tagged_section(&mut result, "temporal_context", None, &items);

        Ok(result)
    }
}

