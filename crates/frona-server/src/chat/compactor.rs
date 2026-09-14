//! Compresses a long conversation so it fits the model's context window:
//! summarize the oldest messages into a rolling `chat_summary` row and load
//! only the messages after the cutoff (`created_at > compacted_until`) plus the
//! summary. Compacted messages are **retained**, not deleted. When a request
//! still can't fit after compaction we do NOT truncate - the provider rejects
//! it (fail loud).
//!
//! Summarization goes through the [`ChatSummarizer`] seam so tests can inject a
//! stub with no live model.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rig_core::completion::Message as RigMessage;

use crate::agent::prompt::PromptLoader;
use crate::chat::message::models::{Message, MessageRole};
use crate::chat::message::repository::MessageRepository;
use crate::chat::models::ChatSummary;
use crate::chat::repository::ChatSummaryRepository;
use crate::core::error::AppError;
use crate::core::repository::{Repository, new_id};
use crate::db::repo::chat_summaries::SurrealChatSummaryRepo;
use crate::db::repo::messages::SurrealMessageRepo;
use crate::inference::context::{estimate_message_tokens, estimate_tokens};
use crate::inference::conversation::{convert_agent_message, format_files_block_simple};
use crate::inference::tool_call::ToolCall;
use crate::inference::usage::{CompactionTarget, InferenceKind, UsageContext, UsageService};
use crate::inference::{ModelProviderRegistry, text_inference};

/// Trigger when the conversation exceeds this fraction of the available window.
const COMPACT_TRIGGER_PCT: usize = 80;
/// Compact down to roughly this fraction (leaves headroom for new turns).
const COMPACT_TARGET_PCT: usize = 70;

/// Result of a compaction-aware load: the rolling summary (if any) plus the
/// messages after the cutoff.
pub struct LoadedConversation {
    pub summary: Option<String>,
    pub messages: Vec<Message>,
}

/// Result of `compact_chat`: whether a new summary was written this call, plus the
/// live conversation (summary + post-cutoff messages) - built from data already in
/// hand, so callers needn't re-`load_conversation` and re-read the history.
pub struct CompactionOutcome {
    pub compacted: bool,
    pub conversation: LoadedConversation,
}

/// The summarization seam. Production wraps `text_inference` (which already
/// retries + falls back); tests inject a stub.
#[async_trait]
pub trait ChatSummarizer: Send + Sync {
    async fn summarize(
        &self,
        user_id: &str,
        agent_id: &str,
        chat_id: &str,
        system_prompt: &str,
        input: &str,
    ) -> Result<String, AppError>;
}

/// Production summarizer: resolves the `compaction` (→`primary`) model group and
/// calls `text_inference`, which carries its own retry/fallback. On exhausted
/// retries it returns `Err`, which `compact_chat` propagates (fail loud).
#[derive(Clone)]
pub struct TextInferenceSummarizer {
    provider_registry: ModelProviderRegistry,
    usage_service: UsageService,
}

impl TextInferenceSummarizer {
    pub fn new(provider_registry: ModelProviderRegistry, usage_service: UsageService) -> Self {
        Self {
            provider_registry,
            usage_service,
        }
    }
}

#[async_trait]
impl ChatSummarizer for TextInferenceSummarizer {
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
        let usage_ctx = UsageContext::new(
            InferenceKind::Compaction {
                target: CompactionTarget::Chat {
                    agent_id: agent_id.to_string(),
                    chat_id: chat_id.to_string(),
                },
            },
            user_id,
            model_group.name.clone(),
        );
        text_inference(
            &self.provider_registry,
            model_group,
            system_prompt,
            vec![RigMessage::user(input)],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        .map_err(|e| AppError::Internal(format!("Chat compaction failed: {e}")))
    }
}

#[derive(Clone)]
pub struct ChatCompactor {
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
    ) -> Self {
        Self {
            chat_summary_repo,
            message_repo,
            summarizer,
            prompts,
        }
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
                Ok(LoadedConversation {
                    summary: Some(summary.content),
                    messages,
                })
            }
            None => Ok(LoadedConversation {
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
        user_id: &str,
        chat_id: &str,
        chat_agent_id: &str,
        system_prompt: &str,
        tool_calls: &[ToolCall],
        context_window: usize,
        max_output_tokens: usize,
    ) -> Result<CompactionOutcome, AppError> {
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
        let existing_summary = existing.as_ref().map(|s| s.content.clone());
        let unchanged = |summary, messages| CompactionOutcome {
            compacted: false,
            conversation: LoadedConversation { summary, messages },
        };
        if live.is_empty() {
            return Ok(unchanged(existing_summary, live));
        }

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
        }
        total_tokens += costs.iter().sum::<usize>();
        if total_tokens <= available * COMPACT_TRIGGER_PCT / 100 {
            return Ok(unchanged(existing_summary, live));
        }

        // Walk newest→oldest keeping a tail under the target; everything older
        // becomes the to-compact prefix.
        let target = available * COMPACT_TARGET_PCT / 100;
        let mut summary_budget = estimate_tokens(system_prompt);
        if let Some(ref s) = existing {
            summary_budget += estimate_tokens(&s.content);
        }
        let mut keep_from_idx = live.len();
        let mut running = 0usize;
        for (i, cost) in costs.iter().enumerate().rev() {
            if running + cost + summary_budget > target {
                break;
            }
            running += cost;
            keep_from_idx = i;
        }
        // The newest message is never compactable. Without this floor, a system
        // prompt (or a single long message) bigger than `target` breaks the loop
        // on its first step, leaving `keep_from_idx` at `live.len()` - every
        // message goes into the summary and the model is handed a conversation
        // with no messages at all, including the question just asked. It then
        // answers that it received nothing.
        keep_from_idx = keep_from_idx.min(live.len().saturating_sub(1));
        if keep_from_idx == 0 {
            return Ok(unchanged(existing_summary, live));
        }
        let messages_to_compact = &live[..keep_from_idx];

        let mut input = String::new();
        if let Some(ref s) = existing {
            input.push_str("Previous summary:\n");
            input.push_str(&s.content);
            input.push_str("\n\nNew messages to incorporate:\n");
        }
        for msg in messages_to_compact {
            let role = match msg.role {
                MessageRole::User => "User",
                MessageRole::Agent => "Agent",
                MessageRole::TaskCompletion => "System",
                MessageRole::Contact => "Contact",
                MessageRole::LiveCall => "Caller",
                MessageRole::System => continue,
            };
            input.push_str(&format!("{role}: {}\n", msg.content));
        }

        let prompt = self.prompts.read("CHAT_COMPACTION.md").ok_or_else(|| {
            AppError::Internal("CHAT_COMPACTION.md prompt missing from resources".into())
        })?;
        let content = self
            .summarizer
            .summarize(user_id, chat_agent_id, chat_id, &prompt, &input)
            .await?;

        let now = Utc::now();
        let compacted_until = messages_to_compact
            .last()
            .map(|m| m.created_at)
            .unwrap_or(now);
        let item_count = messages_to_compact.len() as i64;
        let row = ChatSummary {
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
        if existing.is_some() {
            self.chat_summary_repo.update(&row).await?;
        } else {
            self.chat_summary_repo.create(&row).await?;
        }
        let messages = live.split_off(keep_from_idx);
        Ok(CompactionOutcome {
            compacted: true,
            conversation: LoadedConversation {
                summary: Some(content),
                messages,
            },
        })
    }

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
}

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
    }

    async fn setup(fail: bool) -> (ChatCompactor, SurrealMessageRepo, SurrealChatSummaryRepo) {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        let message_repo: SurrealMessageRepo = SurrealRepo::new(db.clone());
        let summary_repo: SurrealChatSummaryRepo = SurrealRepo::new(db.clone());
        let compactor = ChatCompactor::new(
            SurrealRepo::new(db.clone()),
            SurrealRepo::new(db.clone()),
            Arc::new(StubSummarizer { fail }),
            prompts(),
        );
        (compactor, message_repo, summary_repo)
    }

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
    }

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
            created_at: Utc::now(),
        }
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

        let err = compactor
            .compact_chat("u", chat, "agent", "sys", &[], 200, 0)
            .await;
        assert!(err.is_err(), "summarizer failure must propagate");
        assert!(
            summaries.find_by_chat_id(chat).await.unwrap().is_none(),
            "no partial summary on failure"
        );
    }
}
