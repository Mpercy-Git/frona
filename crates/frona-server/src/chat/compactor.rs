//! Compresses a long conversation so it fits the model's context window:
//! summarize the oldest messages into a rolling `chat_summary` row and load
//! only the messages after the cutoff (`created_at > compacted_until`) plus the
//! summary. Compacted messages are **retained**, not deleted. When a request
//! still cannot fit after compaction we return a limit error and preserve its
//! persisted rows. Initial model context is also checked before inference trimming.
//!
//! Summarization goes through the [`ChatSummarizer`] seam so tests can inject a
//! stub with no live model.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rig_core::completion::request::ToolDefinition as RigToolDefinition;
use rig_core::completion::{AssistantContent, Message as RigMessage};

use crate::agent::prompt::PromptLoader;
use crate::chat::message::models::{Message, MessageCommand, MessageRole};
use crate::chat::message::repository::MessageRepository;
use crate::chat::models::ChatSummary;
use crate::chat::repository::ChatSummaryRepository;
use crate::core::error::AppError;
use crate::core::repository::{Repository, new_id};
use crate::db::repo::chat_summaries::SurrealChatSummaryRepo;
use crate::db::repo::messages::SurrealMessageRepo;
<<<<<<< ours
use crate::inference::context::{estimate_message_tokens, estimate_tokens};
use crate::inference::conversation::{convert_agent_message, format_files_block_simple};
use crate::inference::tool_call::ToolCall;
||||||| base
use crate::inference::context::{estimate_message_tokens, estimate_tokens};
use crate::inference::conversation::{convert_agent_message, format_files_block_simple};
use crate::inference::provider::service::ModelProviderService;
use crate::inference::text_inference;
=======
use crate::db::repo::tool_calls::ToolCallRepository;
use crate::inference::ModelGroup;
use crate::inference::context::{InputTokenEstimate, estimate_message_tokens, estimate_tokens};
use crate::inference::conversation::{ModelConversationHistory, summary_message};
use crate::inference::usage::repository::InferenceUsageRepository;
>>>>>>> upstream
use crate::inference::usage::{CompactionTarget, InferenceKind, UsageContext, UsageService};
use crate::inference::{ModelProviderRegistry, text_inference};

/// Trigger when the conversation exceeds this fraction of the available window.
const COMPACT_TRIGGER_PCT: usize = 80;
/// Compact down to roughly this fraction (leaves headroom for new turns).
const COMPACT_TARGET_PCT: usize = 70;

/// Result of a compaction-aware load: the rolling summary (if any) plus the
/// messages after the cutoff.
pub struct LoadedConversation {
    pub agent_context: Option<String>,
    pub summary: Option<String>,
    pub messages: Vec<Message>,
}

/// Compaction status and the resulting model-facing history.
pub struct CompactionOutcome {
    pub compacted: bool,
    pub model_history: ModelConversationHistory,
    pub input_token_estimate: Option<InputTokenEstimate>,
}

pub struct SummaryRequest<'a> {
    pub input_token_estimate: Option<InputTokenEstimate>,
    pub model_group: &'a ModelGroup,
    pub user_id: &'a str,
    pub agent_id: &'a str,
    pub chat_id: &'a str,
    pub system_prompt: &'a str,
    pub history: Vec<RigMessage>,
    pub tools: Vec<RigToolDefinition>,
}

/// The exact model-facing request prefix; tests can inject deterministic generation.
#[async_trait]
pub trait ChatSummarizer: Send + Sync {
    async fn summarize(&self, request: SummaryRequest<'_>) -> Result<String, AppError>;
}

#[derive(Clone, Copy)]
pub struct CompactionContext<'a> {
    pub agent_context: crate::agent::context::AgentContextScope<'a>,
    pub system_prompt: &'a str,
    pub model_group: &'a ModelGroup,
    pub model_history: &'a ModelConversationHistory,
    pub tools: &'a [RigToolDefinition],
}

/// Generates a summary with the conversation's resolved model group, preserving
/// its request settings, retry/fallback configuration and usage accounting.
/// The selected input prefix is never truncated.
#[derive(Clone)]
pub struct TextInferenceSummarizer {
<<<<<<< ours
    provider_registry: ModelProviderRegistry,
||||||| base
    model_providers: ModelProviderService,
=======
>>>>>>> upstream
    usage_service: UsageService,
}

impl TextInferenceSummarizer {
<<<<<<< ours
    pub fn new(provider_registry: ModelProviderRegistry, usage_service: UsageService) -> Self {
        Self {
            provider_registry,
            usage_service,
        }
||||||| base
    pub fn new(model_providers: ModelProviderService, usage_service: UsageService) -> Self {
        Self {
            model_providers,
            usage_service,
        }
=======
    pub fn new(usage_service: UsageService) -> Self {
        Self { usage_service }
>>>>>>> upstream
    }
}

#[async_trait]
impl ChatSummarizer for TextInferenceSummarizer {
<<<<<<< ours
    async fn summarize(
        &self,
        user_id: &str,
        agent_id: &str,
        chat_id: &str,
        system_prompt: &str,
        input: &str,
    ) -> Result<String, AppError> {
        let model_group = self
            .provider_registry
            .get_model_group("compaction")
            .or_else(|_| self.provider_registry.get_model_group("primary"))
            .map_err(|e| AppError::Internal(format!("No compaction model group: {e}")))?;
||||||| base
    async fn summarize(
        &self,
        user_id: &str,
        agent_id: &str,
        chat_id: &str,
        system_prompt: &str,
        input: &str,
    ) -> Result<String, AppError> {
        let model_group = self
            .model_providers
            .resolve_with_fallback(
                &crate::inference::ModelRef::COMPACTION,
                &crate::inference::ModelRef::PRIMARY,
            )
            .map_err(|e| AppError::Internal(format!("No compaction model group: {e}")))?;
=======
    async fn summarize(&self, request: SummaryRequest<'_>) -> Result<String, AppError> {
        let model_group = request.model_group;
>>>>>>> upstream
        let usage_ctx = UsageContext::new(
            InferenceKind::Compaction {
                target: CompactionTarget::Chat {
                    agent_id: request.agent_id.to_string(),
                    chat_id: request.chat_id.to_string(),
                },
            },
            request.user_id,
            model_group.name.clone(),
        );
<<<<<<< ours
        text_inference(
            model_group,
            system_prompt,
            vec![RigMessage::user(input)],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        .map_err(|e| AppError::Internal(format!("Chat compaction failed: {e}")))
||||||| base
        text_inference(
            &model_group,
            system_prompt,
            vec![RigMessage::user(input)],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        .map_err(|e| AppError::Internal(format!("Chat compaction failed: {e}")))
=======
        let response = model_group
            .inference_preserving_history(crate::inference::ModelRequest {
                input_token_estimate: request.input_token_estimate,
                system_prompt: request.system_prompt,
                history: request.history,
                tools: request.tools,
                usage_service: &self.usage_service,
                usage_context: &usage_ctx,
                overrides: Default::default(),
            })
            .await
            .map_err(|e| AppError::Internal(format!("Chat compaction failed: {e}")))?;
        // This is generation only: definitions preserve the cached prefix, never
        // authorize tool execution. A tool request is not a completed summary.
        if response
            .content
            .iter()
            .any(|item| matches!(item, AssistantContent::ToolCall(_)))
        {
            return Err(AppError::Internal(
                "Chat compaction returned tool calls instead of a summary".into(),
            ));
        }
        let text = response
            .content
            .iter()
            .filter_map(|item| match item {
                AssistantContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            return Err(AppError::Internal(
                "Chat compaction returned an empty summary".into(),
            ));
        }
        Ok(text)
>>>>>>> upstream
    }
}

struct HistoryTokenMeasurement {
    /// Inclusive index of the last stored row covered by the provider usage.
    last_covered_row: usize,
    history_tokens: usize,
}

#[derive(Clone)]
pub struct ChatCompactor {
    agent_context_manager: crate::agent::context::AgentContextManager,
    chat_summary_repo: SurrealChatSummaryRepo,
    message_repo: SurrealMessageRepo,
    summarizer: Arc<dyn ChatSummarizer>,
    prompts: PromptLoader,
}

impl ChatCompactor {
    pub fn new(
        chat_summary_repo: SurrealChatSummaryRepo,
        message_repo: SurrealMessageRepo,
        summarizer: Arc<dyn ChatSummarizer>,
        prompts: PromptLoader,
        agent_context_manager: crate::agent::context::AgentContextManager,
    ) -> Self {
        Self {
            agent_context_manager,
            chat_summary_repo,
            message_repo,
            summarizer,
            prompts,
        }
    }

    /// Usage is optional telemetry. Missing, stale or unreadable statistics must
    /// leave both the trigger and split on their existing estimate-based path.
    async fn load_history_token_measurement(
        &self,
        user_id: &str,
        agent_id: &str,
        live: &[Message],
        context: CompactionContext<'_>,
        summary: Option<&ChatSummary>,
        system_tokens: usize,
    ) -> Option<HistoryTokenMeasurement> {
        let repo = crate::db::repo::generic::SurrealRepo::new(self.message_repo.db().clone());
        let usage = match repo.last_chat_usage(&live.first()?.chat_id).await {
            Ok(usage) => usage?,
            Err(error) => {
                tracing::warn!(%error, "Cannot read chat usage; estimating compaction size");
                return None;
            }
        };
        if usage.user_id != user_id
            || usage.agent_id.as_deref() != Some(agent_id)
            || usage.model_group != context.model_group.name
            || usage.model_ref != context.model_group.main.as_str()
            || usage.provider != context.model_group.main.provider_name()
            || summary.is_some_and(|s| usage.created_at <= s.updated_at)
        {
            return None;
        }
        let anchor = live
            .iter()
            .position(|message| Some(&message.id) == usage.message_id.as_ref())?;
        let message = &live[anchor];
        if message.role != MessageRole::Agent
            || message.agent_id.as_deref() != Some(agent_id)
            || message.status != Some(crate::chat::message::models::MessageStatus::Completed)
        {
            return None;
        }
        let tools =
            crate::db::repo::tool_calls::SurrealToolCallRepo::new(self.message_repo.db().clone())
                .find_by_message_id(&message.id)
                .await
                .ok()?;
        let turns = tools
            .iter()
            .map(|tool| tool.turn)
            .collect::<std::collections::BTreeSet<_>>();
        let complete_tool_history = turns.iter().copied().eq(0..turns.len() as u32);
        match usage.kind_tag.as_str() {
            "Text" if tools.is_empty() && usage.turn_index.is_none() => {}
            "ToolTurn"
                if complete_tool_history && usage.turn_index == u32::try_from(turns.len()).ok() => {
            }
            _ => return None,
        }
        if usage.output_tokens == 0 && (!message.content.is_empty() || message.reasoning.is_some())
        {
            return None;
        }
        // Input is normalized at recording/migration. A missing or inconsistent
        // provider total cannot calibrate estimates; never guess cache counts.
        let input = usage.total_tokens.checked_sub(usage.output_tokens)?;
        if input == 0 || input != usage.input_tokens || input < usage.cached_input_tokens {
            return None;
        }
        let total = usize::try_from(usage.total_tokens).ok()?;
        // A call larger than this model's entire context is not a compatible
        // measurement (including after a configured window reduction).
        if total > context.model_group.context_window {
            return None;
        }
        let history = total
            .checked_sub(system_tokens)
            .filter(|tokens| *tokens > 0)?;
        Some(HistoryTokenMeasurement {
            last_covered_row: anchor,
            history_tokens: history,
        })
    }

    /// Compaction-aware load: the rolling summary + the messages after the
    /// cutoff. Used by the live-context builders (Path A + Path B).
    pub async fn load_conversation(&self, chat_id: &str) -> Result<LoadedConversation, AppError> {
        let existing = self.chat_summary_repo.find_by_chat_id(chat_id).await?;
        match existing {
            Some(summary) => {
                // Fetch only post-cutoff messages in SQL, not the whole history.
                let messages = self
                    .message_repo
                    .find_by_chat_id_after(chat_id, summary.compacted_until)
                    .await?;
                let agent_context = match &summary.agent_context_signature {
                    Some(signature) => {
                        Some(self.agent_context_manager.load(signature).await?.render())
                    }
                    None => None,
                };
                Ok(LoadedConversation {
                    agent_context,
                    summary: Some(summary.content),
                    messages,
                })
            }
            None => Ok(LoadedConversation {
                agent_context: None,
                summary: None,
                messages: self.message_repo.find_by_chat_id(chat_id).await?,
            }),
        }
    }

    /// Summarize the oldest not-yet-compacted messages if the conversation
    /// exceeds the trigger threshold. No-op (returns `false`) when under
    /// threshold. Returns `true` when a new/updated summary was written.
    /// Retains compacted messages (does not delete them).
    // Matches `build_augmented_system_prompt`: the inputs are a flat list of
    // conversation facts, and bundling them into a struct would only move the
    // argument list one call further out.
    #[allow(clippy::too_many_arguments)]
    pub async fn compact_chat(
        &self,
        chat_id: &str,
<<<<<<< ours
        chat_agent_id: &str,
        system_prompt: &str,
        tool_calls: &[ToolCall],
        context_window: usize,
        max_output_tokens: usize,
||||||| base
        chat_agent_id: &str,
        system_prompt: &str,
        context_window: usize,
        max_output_tokens: usize,
=======
        context: CompactionContext<'_>,
>>>>>>> upstream
    ) -> Result<CompactionOutcome, AppError> {
        let CompactionContext {
            agent_context,
            system_prompt,
            model_group,
            model_history,
            tools,
        } = context;
        let user_id = &agent_context.user.id;
        let chat_agent_id = &agent_context.agent.id;
        let context_window = model_group.context_window;
        let max_output_tokens = model_group
            .main
            .request_settings
            .max_tokens
            .unwrap_or(model_group.inference.default_max_tokens)
            as usize;
        let existing = self.chat_summary_repo.find_by_chat_id(chat_id).await?;
        // Only the messages NOT already covered by the summary are live. Fetch just
        // those in SQL - the compacted rest is retained on disk but represented by
        // the summary, so we never re-read (or re-compact) it each turn.
        let mut live: Vec<Message> = match &existing {
            Some(s) => {
                self.message_repo
                    .find_by_chat_id_after(chat_id, s.compacted_until)
                    .await?
            }
            None => self.message_repo.find_by_chat_id(chat_id).await?,
        };
        let unchanged = |input_token_estimate| CompactionOutcome {
            compacted: false,
            model_history: model_history.clone(),
            input_token_estimate,
        };
        if live.is_empty() {
            return Ok(unchanged(None));
        }

<<<<<<< ours
        // One cost per live message, index-aligned with `live`. A row the builder
        // drops (a System note, an agent turn still executing) costs nothing but
        // must keep its slot: the split point below indexes into `live`, so a
        // filtered-out row used to shift it and compact a message more than
        // intended.
        let costs: Vec<usize> = live
            .iter()
            .map(|msg| {
                let body = Self::to_rig(msg, chat_agent_id)
                    .as_ref()
                    .map(estimate_message_tokens)
                    .unwrap_or(0);
                body + tool_turn_tokens(&msg.id, tool_calls)
            })
            .collect();
        let available = context_window.saturating_sub(max_output_tokens);

        let mut total_tokens = estimate_tokens(system_prompt);
        if let Some(ref s) = existing {
            total_tokens += estimate_tokens(&s.content);
||||||| base
        let rig_messages = self.to_rig(&live, chat_agent_id);
        let available = context_window.saturating_sub(max_output_tokens);

        let mut total_tokens = estimate_tokens(system_prompt);
        if let Some(ref s) = existing {
            total_tokens += estimate_tokens(&s.content);
=======
        // A resumed response may intentionally exclude later incoming rows.
        // The persisted prefix must still match exactly; future text must not
        // enter this response's summary or request.
        let has_later_rows = model_history.row_ids.len() < live.len();
        if model_history.row_ids.len() < live.len()
            && model_history
                .row_ids
                .iter()
                .eq(live.iter().take(model_history.row_ids.len()).map(|m| &m.id))
        {
            live.truncate(model_history.row_ids.len());
        }
        if live.is_empty() {
            return Ok(unchanged(None));
>>>>>>> upstream
        }
<<<<<<< ours
        total_tokens += costs.iter().sum::<usize>();
||||||| base
        for msg in &rig_messages {
            total_tokens += estimate_message_tokens(msg);
        }
=======

        // These offsets came from the normal conversation builder. Validate the
        // source before using them as a database cutoff; a stale/reordered request
        // must never summarize a different set of persisted rows.
        model_history.validate_source(
            &live,
            existing.as_ref().map(|summary| summary.content.as_str()),
        )?;
        let mut costs: Vec<usize> = (0..live.len())
            .map(|row| {
                model_history.history[model_history.row_range(row)]
                    .iter()
                    .map(estimate_message_tokens)
                    .sum()
            })
            .collect();
        // Builder-injected continuation/heartbeat messages have no database row.
        // They remain protected with the tail and never enter the summarized prefix.
        let stored_end = model_history.row_range(live.len() - 1).end;
        let injected_tokens = model_history.history[stored_end..]
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>();
        let available = context_window.saturating_sub(max_output_tokens);
        let system_tokens = estimate_tokens(system_prompt)
            + tools
                .iter()
                .map(|tool| {
                    estimate_tokens(
                        &serde_json::to_string(tool).expect("tool definition is serializable"),
                    )
                })
                .sum::<usize>();
        let mut summary_tokens = model_history.history[..model_history.summary_end]
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>();
        // Calibrate only the prefix actually covered by the last completed call.
        // Its input already includes prior tool turns; adding the whole owning
        // assistant row again would double-count them. New rows stay estimated.
        let calibrated = if let Some(HistoryTokenMeasurement {
            last_covered_row,
            history_tokens,
        }) = self
            .load_history_token_measurement(
                user_id,
                chat_agent_id,
                &live,
                context,
                existing.as_ref(),
                system_tokens,
            )
            .await
        {
            let estimated_history =
                summary_tokens + costs[..=last_covered_row].iter().sum::<usize>();
            let fits_counter = history_tokens
                .checked_add(system_tokens)
                .and_then(|total| {
                    total.checked_add(costs[last_covered_row + 1..].iter().sum::<usize>())
                })
                .and_then(|total| total.checked_add(injected_tokens))
                .and_then(|total| total.checked_add(costs.len() + 1))
                .is_some();
            if estimated_history > 0 && fits_counter {
                let scale = |tokens: usize| -> usize {
                    ((tokens as u128 * history_tokens as u128).div_ceil(estimated_history as u128))
                        .min(usize::MAX as u128) as usize
                };
                summary_tokens = scale(summary_tokens);
                for cost in &mut costs[..=last_covered_row] {
                    *cost = scale(*cost);
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        if let Some(last) = costs.last_mut() {
            *last += injected_tokens;
        }
        let total_tokens = system_tokens + summary_tokens + costs.iter().sum::<usize>();
        let input_token_estimate = calibrated.then(|| {
            InputTokenEstimate::new(
                total_tokens,
                &model_group.main,
                system_prompt,
                &model_history.history,
                tools,
            )
        });
>>>>>>> upstream
        if total_tokens <= available * COMPACT_TRIGGER_PCT / 100 {
            return Ok(unchanged(input_token_estimate));
        }

        // An empty assistant placeholder (or a slash-command acknowledgement)
        // must never move the protection boundary past the actual request.
        let protected = live
            .iter()
            .rposition(is_substantive_request)
            .unwrap_or(live.len() - 1);
        let protected = timestamp_group_start(&live, protected);
        let protected_tokens = costs[protected..].iter().sum::<usize>();
        ensure_fits(system_tokens + protected_tokens, available)?;

        let target = available * COMPACT_TARGET_PCT / 100;
<<<<<<< ours
        let mut summary_budget = estimate_tokens(system_prompt);
        if let Some(ref s) = existing {
            summary_budget += estimate_tokens(&s.content);
        }
        let mut keep_from_idx = live.len();
        let mut running = 0usize;
        for (i, cost) in costs.iter().enumerate().rev() {
            if running + cost + summary_budget > target {
||||||| base
        let mut summary_budget = estimate_tokens(system_prompt);
        if let Some(ref s) = existing {
            summary_budget += estimate_tokens(&s.content);
        }
        let mut keep_from_idx = live.len();
        let mut running = 0usize;
        for (i, msg) in rig_messages.iter().enumerate().rev() {
            let cost = estimate_message_tokens(msg);
            if running + cost + summary_budget > target {
=======
        let mut keep_from_idx = protected;
        let mut running = protected_tokens;
        for i in (0..protected).rev() {
            if running + costs[i] + system_tokens + summary_tokens > target {
>>>>>>> upstream
                break;
            }
            running += costs[i];
            keep_from_idx = i;
        }
<<<<<<< ours
        // Protect the current request and every following response/tool row.
        // The web path creates an empty executing assistant placeholder before
        // loading this history, so keeping only the newest row can summarize
        // away the actual question (and its image), leaving just a placeholder
        // that the conversation builder then skips.
        let request_idx = live
            .iter()
            .rposition(|msg| {
                matches!(
                    msg.role,
                    MessageRole::User
                        | MessageRole::Contact
                        | MessageRole::LiveCall
                        | MessageRole::TaskCompletion
                ) && (!msg.content.trim().is_empty() || !msg.attachments.is_empty())
            })
            .unwrap_or(live.len() - 1);
        // Reload uses a strict timestamp cutoff. Keep all rows sharing the
        // request's timestamp so a compacted sibling cannot hide it on resume.
        let protected_from = live
            .iter()
            .position(|msg| msg.created_at >= live[request_idx].created_at)
            .unwrap_or(request_idx);
        keep_from_idx = keep_from_idx.min(protected_from);
||||||| base
=======
        // The existing persisted cutoff is timestamp-only and exclusive when
        // loading. Keep the entire boundary timestamp group so reload cannot
        // silently discard a retained row with the same timestamp as the prefix.
        keep_from_idx = timestamp_group_start(&live, keep_from_idx);
>>>>>>> upstream
        if keep_from_idx == 0 {
            ensure_fits(total_tokens, available)?;
            return Ok(unchanged(input_token_estimate));
        }
        if has_later_rows {
            return Err(AppError::Conflict(
                "Cannot compact an earlier response while newer messages are pending".into(),
            ));
        }
        // Record when the fresh baseline was captured, before source collection and inference.
        let now = live
            .last()
            .map_or_else(Utc::now, |message| Utc::now().max(message.created_at));
        // Fresh sources must all succeed before even the summarizer runs.
        let chat = crate::db::repo::chats::SurrealChatRepo::new(self.message_repo.db().clone())
            .find_by_id(chat_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Chat".into()))?;
        let mut agent_context_state = chat.agent_context_state.clone();
        let snapshot = self
            .agent_context_manager
            .prepare(&agent_context, None, &mut agent_context_state)
            .await?
            .snapshot;
        let context_text = snapshot.render();
        let has_agent_context = !context_text.is_empty();
        let messages_to_compact = &live[..keep_from_idx];

        let prefix_end = model_history.row_range(keep_from_idx - 1).end;
        let mut history = model_history.history[..prefix_end].to_vec();
        let instruction = self.prompts.read("CHAT_COMPACTION.md").ok_or_else(|| {
            AppError::Internal("CHAT_COMPACTION.md prompt missing from resources".into())
        })?;
        history.push(RigMessage::user(instruction));
        let summary_input_tokens = system_tokens
            + summary_tokens
            + costs[..keep_from_idx].iter().sum::<usize>()
            + estimate_message_tokens(history.last().expect("summary instruction"));
        let content = self
            .summarizer
            .summarize(SummaryRequest {
                input_token_estimate: calibrated.then(|| {
                    InputTokenEstimate::new(
                        summary_input_tokens,
                        &model_group.main,
                        system_prompt,
                        &history,
                        tools,
                    )
                }),
                model_group,
                user_id,
                agent_id: chat_agent_id,
                chat_id,
                system_prompt,
                history,
                tools: tools.to_vec(),
            })
            .await?;
        if content.trim().is_empty() {
            return Err(AppError::Internal(
                "Chat compaction returned an empty summary".into(),
            ));
        }

        // Do not publish a summary that still cannot fit alongside the retained
        // request. The original rows and prior summary remain recoverable on error.
        if !has_agent_context {
            ensure_fits(
                system_tokens
                    + summary_tokens_for(&content)
                    + costs[keep_from_idx..].iter().sum::<usize>(),
                available,
            )?;
        }
        let mut compacted_history = model_history.clone();
        compacted_history.replace_prefix(keep_from_idx, &content);
        compacted_history.set_summary_context(Some(&context_text));
        let compacted_until = messages_to_compact
            .last()
            .map(|m| m.created_at)
            .unwrap_or(now);
        let item_count = messages_to_compact.len() as i64;
        let row = ChatSummary {
            agent_context_signature: Some(snapshot.signature),
            id: existing
                .as_ref()
                .map(|s| s.id.clone())
                .unwrap_or_else(new_id),
            user_id: user_id.to_string(),
            chat_id: chat_id.to_string(),
            content: content.clone(),
            compacted_until,
            item_count,
            created_at: existing.as_ref().map(|s| s.created_at).unwrap_or(now),
            updated_at: now,
        };
        if has_agent_context {
            validate_initial_context(
                &compacted_history.history,
                system_prompt,
                model_group,
                tools,
                None,
            )?;
        }
        if existing.is_some() {
            self.chat_summary_repo.update(&row).await?;
        } else {
            self.chat_summary_repo.create(&row).await?;
        }
        if agent_context_state != chat.agent_context_state {
            crate::db::repo::chats::SurrealChatRepo::new(self.message_repo.db().clone())
                .save_agent_context_state(chat_id, &agent_context_state)
                .await?;
        }
        let input_token_estimate = (calibrated && !has_agent_context).then(|| {
            InputTokenEstimate::new(
                system_tokens
                    + summary_tokens_for(&content)
                    + costs[keep_from_idx..].iter().sum::<usize>(),
                &model_group.main,
                system_prompt,
                &compacted_history.history,
                tools,
            )
        });
        Ok(CompactionOutcome {
            input_token_estimate,
            compacted: true,
            model_history: compacted_history,
        })
    }
<<<<<<< ours

    /// Approximate one rig message for token *estimation* only - coarser than the
    /// conversation builder (roles grouped). `System` rows drop. What the builder
    /// replays around an agent message - its turn text, its tool calls and their
    /// results - is counted separately by [`tool_turn_tokens`].
    fn to_rig(msg: &Message, chat_agent_id: &str) -> Option<RigMessage> {
        match msg.role {
            MessageRole::User | MessageRole::TaskCompletion | MessageRole::Contact => {
                let content = format_files_block_simple(&msg.content, &msg.attachments);
                Some(RigMessage::user(&content))
            }
            MessageRole::LiveCall => {
                let content = format_files_block_simple(&msg.content, &msg.attachments);
                Some(RigMessage::user(format!("[LIVE_CALL] {content}")))
            }
            MessageRole::Agent => convert_agent_message(msg, chat_agent_id, None),
            MessageRole::System => None,
        }
    }
||||||| base

    /// Approximate rig messages for token *estimation* only - coarser than the
    /// conversation builder (roles grouped, tool calls omitted). `System` rows drop.
    fn to_rig(&self, messages: &[Message], chat_agent_id: &str) -> Vec<RigMessage> {
        messages
            .iter()
            .filter_map(|msg| match msg.role {
                MessageRole::User | MessageRole::TaskCompletion | MessageRole::Contact => {
                    let content = format_files_block_simple(&msg.content, &msg.attachments);
                    Some(RigMessage::user(&content))
                }
                MessageRole::LiveCall => {
                    let content = format_files_block_simple(&msg.content, &msg.attachments);
                    Some(RigMessage::user(format!("[LIVE_CALL] {content}")))
                }
                MessageRole::Agent => convert_agent_message(msg, chat_agent_id, None),
                MessageRole::System => None,
            })
            .collect()
    }
=======
>>>>>>> upstream
}

<<<<<<< ours
/// What one message's tool turns add to the request.
///
/// The builder replays every tool call of an agent message - the assistant's turn
/// text, each call's arguments, and each result - and none of it was being counted
/// here. On an agentic turn that is most of the payload: a handful of searches and
/// file reads dwarf the prose around them, so the conversation could sit far over
/// the window while this estimate still read "under threshold" and compaction
/// declined to run.
fn tool_turn_tokens(message_id: &str, tool_calls: &[ToolCall]) -> usize {
    tool_calls
        .iter()
        .filter(|call| call.message_id == message_id)
        .map(|call| {
            estimate_tokens(&call.result)
                + estimate_tokens(&call.arguments.to_string())
                + call.turn_text.as_deref().map(estimate_tokens).unwrap_or(0)
                + call
                    .turn_reasoning
                    .as_ref()
                    .map(|r| {
                        estimate_tokens(&r.content)
                            + r.raw
                                .as_ref()
                                .map(|raw| estimate_tokens(&raw.to_string()))
                                .unwrap_or(0)
                    })
                    .unwrap_or(0)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use chrono::Duration;
    use surrealdb::Surreal;
    use surrealdb::engine::local::Mem;

    use crate::chat::repository::ChatSummaryRepository;
    use crate::db::repo::chat_summaries::SurrealChatSummaryRepo;
    use crate::db::repo::generic::SurrealRepo;
    use crate::db::repo::messages::SurrealMessageRepo;

    /// Returns `[SUMMARY]`, or errors when `fail` - no live model.
    struct StubSummarizer {
        fail: bool,
    }

    #[async_trait]
    impl ChatSummarizer for StubSummarizer {
        async fn summarize(
            &self,
            _user_id: &str,
            _agent_id: &str,
            _chat_id: &str,
            _system_prompt: &str,
            _input: &str,
        ) -> Result<String, AppError> {
            if self.fail {
                Err(AppError::Internal("stub summarizer failure".into()))
            } else {
                Ok("[SUMMARY]".into())
            }
        }
    }

    fn prompts() -> PromptLoader {
        PromptLoader::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("resources")
                .join("prompts"),
        )
||||||| base
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use chrono::Duration;
    use surrealdb::Surreal;
    use surrealdb::engine::local::Mem;

    use crate::chat::repository::ChatSummaryRepository;
    use crate::db::repo::chat_summaries::SurrealChatSummaryRepo;
    use crate::db::repo::generic::SurrealRepo;
    use crate::db::repo::messages::SurrealMessageRepo;

    /// Returns `[SUMMARY]`, or errors when `fail` - no live model.
    struct StubSummarizer {
        fail: bool,
    }

    #[async_trait]
    impl ChatSummarizer for StubSummarizer {
        async fn summarize(
            &self,
            _user_id: &str,
            _agent_id: &str,
            _chat_id: &str,
            _system_prompt: &str,
            _input: &str,
        ) -> Result<String, AppError> {
            if self.fail {
                Err(AppError::Internal("stub summarizer failure".into()))
            } else {
                Ok("[SUMMARY]".into())
            }
        }
    }

    fn prompts() -> PromptLoader {
        PromptLoader::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("resources")
                .join("prompts"),
        )
=======
/// Validate the actual, fully built initial context against the budget used by
/// inference's truncation layer. Compaction estimates are deliberately coarse;
/// this final check prevents initial requests/attachments being silently trimmed.
/// This covers the initial main-model request. Fallback budgets and tool-loop
/// growth after this boundary are handled separately by inference.
pub(crate) fn validate_initial_context(
    history: &[RigMessage],
    system_prompt: &str,
    model_group: &crate::inference::ModelGroup,
    tools: &[RigToolDefinition],
    input_token_estimate: Option<InputTokenEstimate>,
) -> Result<(), AppError> {
    if let Some(tokens) = input_token_estimate
        .and_then(|estimate| estimate.for_request(&model_group.main, system_prompt, history, tools))
    {
        let output = model_group
            .main
            .request_settings
            .max_tokens
            .unwrap_or(model_group.inference.default_max_tokens) as usize;
        return ensure_fits(tokens, model_group.context_window.saturating_sub(output));
>>>>>>> upstream
    }
    let tokens = history.iter().map(estimate_message_tokens).sum::<usize>();
    let model = &model_group.main;
    let max_output = model
        .request_settings
        .max_tokens
        .unwrap_or(model_group.inference.default_max_tokens) as usize;
    let budget = model_group
        .context_window
        .saturating_sub(max_output)
        .saturating_sub(estimate_tokens(system_prompt))
        * model_group.inference.history_truncation_pct
        / 100;
    if tokens > budget {
        return Err(AppError::Validation(format!(
            "Chat context exceeds the configured history limit ({tokens} estimated tokens, {budget} available); the current request has been preserved"
        )));
    }
    Ok(())
}

<<<<<<< ours
    async fn seed(repo: &SurrealMessageRepo, chat_id: &str, n: usize, len: usize) {
        let base = Utc::now() - Duration::hours(1);
        for i in 0..n {
            let mut m = Message::builder(chat_id, MessageRole::User, "x".repeat(len)).build();
            m.id = new_id();
            m.created_at = base + Duration::seconds(i as i64);
            repo.create(&m).await.unwrap();
        }
    }

    #[tokio::test]
    async fn compacts_persists_retains_and_loads_cutoff() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-1";
        seed(&messages, chat, 6, 200).await;

        let out = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 200, 0)
            .await
            .unwrap();
        assert!(out.compacted, "expected compaction to run");

        let summary = summaries.find_by_chat_id(chat).await.unwrap().unwrap();
        assert_eq!(summary.content, "[SUMMARY]");
        assert!(summary.item_count > 0 && summary.item_count < 6);

        let all = messages.find_by_chat_id(chat).await.unwrap();
        assert_eq!(all.len(), 6, "compacted messages must be retained");

        let loaded = compactor.load_conversation(chat).await.unwrap();
        assert_eq!(loaded.summary.as_deref(), Some("[SUMMARY]"));
        assert_eq!(loaded.messages.len(), 6 - summary.item_count as usize);
        assert!(
            loaded
                .messages
                .iter()
                .all(|m| m.created_at > summary.compacted_until)
        );

        assert_eq!(out.conversation.summary, loaded.summary);
        assert_eq!(
            out.conversation
                .messages
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
            loaded
                .messages
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
        );
    }

    #[tokio::test]
    async fn no_op_under_threshold() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-2";
        seed(&messages, chat, 1, 20).await;

        let out = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 100_000, 0)
            .await
            .unwrap();
        assert!(!out.compacted);
        assert!(summaries.find_by_chat_id(chat).await.unwrap().is_none());
        assert!(out.conversation.summary.is_none());
        assert_eq!(out.conversation.messages.len(), 1);

        let loaded = compactor.load_conversation(chat).await.unwrap();
        assert!(loaded.summary.is_none());
        assert_eq!(loaded.messages.len(), 1);
    }

    #[tokio::test]
    async fn prior_summary_returns_summary_and_post_cutoff() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-4";
        seed(&messages, chat, 6, 200).await;

        let first = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 200, 0)
            .await
            .unwrap();
        assert!(first.compacted);
        let summary = summaries.find_by_chat_id(chat).await.unwrap().unwrap();
||||||| base
    async fn seed(repo: &SurrealMessageRepo, chat_id: &str, n: usize, len: usize) {
        let base = Utc::now() - Duration::hours(1);
        for i in 0..n {
            let mut m = Message::builder(chat_id, MessageRole::User, "x".repeat(len)).build();
            m.id = new_id();
            m.created_at = base + Duration::seconds(i as i64);
            repo.create(&m).await.unwrap();
        }
    }

    #[tokio::test]
    async fn compacts_persists_retains_and_loads_cutoff() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-1";
        seed(&messages, chat, 6, 200).await;

        let out = compactor
            .compact_chat("u", chat, "agent", "sys", 200, 0)
            .await
            .unwrap();
        assert!(out.compacted, "expected compaction to run");

        let summary = summaries.find_by_chat_id(chat).await.unwrap().unwrap();
        assert_eq!(summary.content, "[SUMMARY]");
        assert!(summary.item_count > 0 && summary.item_count < 6);

        let all = messages.find_by_chat_id(chat).await.unwrap();
        assert_eq!(all.len(), 6, "compacted messages must be retained");

        let loaded = compactor.load_conversation(chat).await.unwrap();
        assert_eq!(loaded.summary.as_deref(), Some("[SUMMARY]"));
        assert_eq!(loaded.messages.len(), 6 - summary.item_count as usize);
        assert!(
            loaded
                .messages
                .iter()
                .all(|m| m.created_at > summary.compacted_until)
        );

        assert_eq!(out.conversation.summary, loaded.summary);
        assert_eq!(
            out.conversation
                .messages
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
            loaded
                .messages
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
        );
    }

    #[tokio::test]
    async fn no_op_under_threshold() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-2";
        seed(&messages, chat, 1, 20).await;

        let out = compactor
            .compact_chat("u", chat, "agent", "sys", 100_000, 0)
            .await
            .unwrap();
        assert!(!out.compacted);
        assert!(summaries.find_by_chat_id(chat).await.unwrap().is_none());
        assert!(out.conversation.summary.is_none());
        assert_eq!(out.conversation.messages.len(), 1);

        let loaded = compactor.load_conversation(chat).await.unwrap();
        assert!(loaded.summary.is_none());
        assert_eq!(loaded.messages.len(), 1);
    }

    #[tokio::test]
    async fn prior_summary_returns_summary_and_post_cutoff() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-4";
        seed(&messages, chat, 6, 200).await;

        let first = compactor
            .compact_chat("u", chat, "agent", "sys", 200, 0)
            .await
            .unwrap();
        assert!(first.compacted);
        let summary = summaries.find_by_chat_id(chat).await.unwrap().unwrap();
=======
fn summary_tokens_for(content: &str) -> usize {
    estimate_message_tokens(&summary_message(content))
}
>>>>>>> upstream

<<<<<<< ours
        let second = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 100_000, 0)
            .await
            .unwrap();
        assert!(!second.compacted, "must not re-compact under threshold");
        assert_eq!(second.conversation.summary.as_deref(), Some("[SUMMARY]"));
        assert_eq!(
            second.conversation.messages.len(),
            6 - summary.item_count as usize
        );
        assert!(
            second
                .conversation
                .messages
                .iter()
                .all(|m| m.created_at > summary.compacted_until)
        );
||||||| base
        let second = compactor
            .compact_chat("u", chat, "agent", "sys", 100_000, 0)
            .await
            .unwrap();
        assert!(!second.compacted, "must not re-compact under threshold");
        assert_eq!(second.conversation.summary.as_deref(), Some("[SUMMARY]"));
        assert_eq!(
            second.conversation.messages.len(),
            6 - summary.item_count as usize
        );
        assert!(
            second
                .conversation
                .messages
                .iter()
                .all(|m| m.created_at > summary.compacted_until)
        );
=======
fn ensure_fits(tokens: usize, available: usize) -> Result<(), AppError> {
    if tokens > available {
        return Err(AppError::Validation(format!(
            "Chat context exceeds the available context limit ({tokens} estimated tokens, {available} available); the current request has been preserved"
        )));
>>>>>>> upstream
    }
    Ok(())
}

<<<<<<< ours
    fn tool_call(message_id: &str, result_len: usize) -> ToolCall {
        ToolCall {
            id: new_id(),
            chat_id: "chat-1".into(),
            message_id: message_id.into(),
            turn: 1,
            provider_call_id: "p1".into(),
            name: "memory_search".into(),
            arguments: serde_json::json!({"query": "motion sensors"}),
            result: "x".repeat(result_len),
            success: true,
            duration_ms: 1,
            hitl: None,
            task_event: None,
            system_prompt: None,
            description: None,
            turn_text: None,
            turn_reasoning: None,
            signature: None,
            created_at: Utc::now(),
        }
    }

    async fn image_request_with_placeholder(repo: &SurrealMessageRepo, chat: &str) -> Message {
        let request = Message::builder(
            chat,
            MessageRole::User,
            "Identify the object in this image and tell me its colour.".into(),
        )
        .attachments(vec![crate::storage::Attachment {
            filename: "object.jpg".into(),
            content_type: "image/jpeg".into(),
            size_bytes: 100,
            owner: "user:u".into(),
            path: "object.jpg".into(),
            url: None,
            transcript: None,
        }])
        .build();
        repo.create(&request).await.unwrap();
        let mut placeholder = Message::builder(chat, MessageRole::Agent, String::new())
            .agent_id("agent".into())
            .status(crate::chat::message::models::MessageStatus::Executing)
            .build();
        placeholder.created_at = request.created_at + Duration::seconds(1);
        repo.create(&placeholder).await.unwrap();
        request
    }

    #[tokio::test]
    async fn first_image_request_is_not_compacted_in_favour_of_an_empty_placeholder() {
        // No old conversation exists to summarize. The failing summarizer
        // proves we don't try to compact the current request even over budget.
        let (compactor, messages, summaries) = setup(true).await;
        let chat = "chat-first-image";
        let request = image_request_with_placeholder(&messages, chat).await;
        let out = compactor
            .compact_chat("u", chat, "agent", &"s".repeat(4000), &[], 1200, 0)
            .await
            .unwrap();
        assert!(!out.compacted);
        assert_eq!(out.conversation.messages.first(), Some(&request));
        assert!(summaries.find_by_chat_id(chat).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn image_request_and_placeholder_survive_compaction_and_reload() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-current-image";
        seed(&messages, chat, 4, 200).await;
        let request = image_request_with_placeholder(&messages, chat).await;
        let out = compactor
            .compact_chat("u", chat, "agent", &"s".repeat(4000), &[], 1200, 0)
            .await
            .unwrap();
        assert!(out.compacted);
        assert_eq!(out.conversation.messages.first(), Some(&request));
        assert_eq!(out.conversation.messages.len(), 2);
        let summary = summaries.find_by_chat_id(chat).await.unwrap().unwrap();
        assert!(summary.compacted_until < request.created_at);
        let loaded = compactor.load_conversation(chat).await.unwrap();
        assert_eq!(loaded.messages, out.conversation.messages);
    }

    #[tokio::test]
    async fn a_timestamp_shared_with_the_request_never_becomes_the_cutoff() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-shared-timestamp";
        seed(&messages, chat, 4, 200).await;
        let request = image_request_with_placeholder(&messages, chat).await;
        let mut sibling =
            Message::builder(chat, MessageRole::System, "context note".into()).build();
        sibling.created_at = request.created_at;
        messages.create(&sibling).await.unwrap();
        let out = compactor
            .compact_chat("u", chat, "agent", &"s".repeat(4000), &[], 1200, 0)
            .await
            .unwrap();
        assert!(out.compacted);
        let summary = summaries.find_by_chat_id(chat).await.unwrap().unwrap();
        assert!(summary.compacted_until < request.created_at);
        let loaded = compactor.load_conversation(chat).await.unwrap();
        assert!(loaded.messages.contains(&request));
        assert!(loaded.messages.contains(&sibling));
    }

    /// The failure this guards against is the worst one the loop can produce: the
    /// agent replying that it received no question, because it genuinely didn't.
    /// A system prompt larger than the compaction target used to break the
    /// newest-first walk on its first step, so every message - the one just sent
    /// included - went into the summary and the model got an empty conversation.
    #[tokio::test]
    async fn the_newest_message_survives_a_system_prompt_larger_than_the_target() {
        let (compactor, messages, _) = setup(false).await;
        let chat = "chat-wipe";
        seed(&messages, chat, 4, 200).await;

        // 4k chars ≈ 1k tokens of system prompt against a 1.2k-token window:
        // bigger than target (70%), which is the regime a memory manual plus
        // injected playbooks and short memory actually puts us in.
        let out = compactor
            .compact_chat("u", chat, "agent", &"s".repeat(4000), &[], 1200, 0)
            .await
            .unwrap();

        assert!(
            !out.conversation.messages.is_empty(),
            "the question just asked must reach the model"
        );
        let all = messages.find_by_chat_id(chat).await.unwrap();
        assert_eq!(
            out.conversation.messages.last().map(|m| m.id.clone()),
            all.last().map(|m| m.id.clone()),
            "and the message that survives is the newest one"
        );
    }

    /// Tool results are most of an agentic turn's payload. Counting only the prose
    /// let a conversation sit far over the window while this read "under threshold".
    #[tokio::test]
    async fn tool_results_count_towards_the_threshold() {
        let (compactor, messages, _) = setup(false).await;
        let chat = "chat-tools";
        seed(&messages, chat, 3, 100).await;
        let all = messages.find_by_chat_id(chat).await.unwrap();

        let without = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 4000, 0)
            .await
            .unwrap();
        assert!(!without.compacted, "prose alone is well under the window");

        let calls: Vec<ToolCall> = all.iter().map(|m| tool_call(&m.id, 8000)).collect();
        let with = compactor
            .compact_chat("u", chat, "agent", "sys", &calls, 4000, 0)
            .await
            .unwrap();
        assert!(
            with.compacted,
            "the same conversation with 24k chars of tool results does not fit"
        );
    }

    /// `keep_from_idx` indexes into the stored messages, but was computed over a
    /// filtered list. One dropped row (a System note) shifted the split by one and
    /// swallowed a message that should have stayed live.
    #[tokio::test]
    async fn a_filtered_row_does_not_shift_the_split_point() {
        let (compactor, messages, summaries) = setup(false).await;
        let chat = "chat-filtered";
        let base = Utc::now() - Duration::hours(1);
        for i in 0..6 {
            // Every other row is a System note, which the builder drops.
            let role = if i % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::System
            };
            let mut m = Message::builder(chat, role, "x".repeat(200)).build();
            m.id = new_id();
            m.created_at = base + Duration::seconds(i);
            messages.create(&m).await.unwrap();
        }

        let out = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 200, 0)
            .await
            .unwrap();
        assert!(out.compacted);

        let summary = summaries.find_by_chat_id(chat).await.unwrap().unwrap();
        let all = messages.find_by_chat_id(chat).await.unwrap();
        // Live and compacted must partition the conversation exactly at the cutoff.
        assert_eq!(
            out.conversation.messages.len(),
            all.iter()
                .filter(|m| m.created_at > summary.compacted_until)
                .count(),
            "the split lands on the message the summary says it does"
        );
        assert!(
            out.conversation
                .messages
                .iter()
                .all(|m| m.created_at > summary.compacted_until)
        );
    }

    #[tokio::test]
    async fn fail_loud_on_summarizer_error() {
        let (compactor, messages, summaries) = setup(true).await;
        let chat = "chat-3";
        seed(&messages, chat, 6, 200).await;
||||||| base
    #[tokio::test]
    async fn fail_loud_on_summarizer_error() {
        let (compactor, messages, summaries) = setup(true).await;
        let chat = "chat-3";
        seed(&messages, chat, 6, 200).await;
=======
fn is_substantive_request(msg: &Message) -> bool {
    matches!(
        msg.role,
        MessageRole::User
            | MessageRole::Contact
            | MessageRole::LiveCall
            | MessageRole::TaskCompletion
    ) && !matches!(msg.command, Some(MessageCommand::Command { .. }))
        && (!msg.content.trim().is_empty() || !msg.attachments.is_empty())
}
>>>>>>> upstream

<<<<<<< ours
        let err = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 200, 0)
            .await;
        assert!(err.is_err(), "summarizer failure must propagate");
        assert!(
            summaries.find_by_chat_id(chat).await.unwrap().is_none(),
            "no partial summary on failure"
        );
||||||| base
        let err = compactor
            .compact_chat("u", chat, "agent", "sys", 200, 0)
            .await;
        assert!(err.is_err(), "summarizer failure must propagate");
        assert!(
            summaries.find_by_chat_id(chat).await.unwrap().is_none(),
            "no partial summary on failure"
        );
=======
fn timestamp_group_start(messages: &[Message], mut index: usize) -> usize {
    while index > 0 && messages[index - 1].created_at == messages[index].created_at {
        index -= 1;
>>>>>>> upstream
    }
    index
}
