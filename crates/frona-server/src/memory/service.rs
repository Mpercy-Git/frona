//! The memory-system seam.
//!
//! A [`MemoryService`] is the abstraction over *how memory works* - the
//! foreground-facing surface only. Exactly one service is selected at boot from
//! config (`basic` or `pkm`); switching requires a restart, so the
//! trait is deliberately not designed for hot-swap.
//!
//! Two things are intentionally **not** on this trait:
//!
//! * **Background maintenance / consolidation.** Each implementation owns its
//!   own curation and wires its own triggers (chat-end, idle, scheduler cron)
//!   at construction. The trait says nothing about it.
//! * **A fixed "retrieval result" shape.** Rather than returning a single
//!   string to splice, [`MemoryService::retrieve`] is handed a mutable
//!   [`MemoryContext`] and decides what to do with it - append a block to the
//!   system-prompt tail, insert RAG context inline into the message history,
//!   rerank, etc. This keeps a future per-message RAG service possible without
//!   reshaping the trait.

use std::sync::Arc;

use async_trait::async_trait;
use rig_core::completion::Message as RigMessage;

use crate::core::error::AppError;
use crate::inference::InferenceContext;
use crate::tool::AgentTool;

/// A narrowed, mutable view of an in-flight turn, handed to
/// [`MemoryService::retrieve`].
///
/// This is intentionally *not* the whole `InferenceRequest`: a memory service
/// has no business touching the tool registry, model group, provider registry,
/// or usage accounting, and giving it `&mut` to those is a footgun. It gets
/// exactly the two fields it legitimately mutates plus read-only context.
///
/// **Caching contract (not structurally enforced):** the static, cacheable head
/// of the system prompt is assembled *before* `retrieve` runs. Implementations
/// must only **append to the tail** of `system_prompt` and/or mutate `history` -
/// never rewrite the head, or they break provider prefix caching. To stay
/// cacheable, append any *constant* usage instructions **first**, before the
/// per-turn dynamic blocks, so `[head][your static section]` remains a stable
/// prefix and only the dynamic tail falls outside the cache.
pub struct MemoryContext<'a> {
    /// The fully-assembled system prompt. Append the dynamic memory block here.
    pub system_prompt: &'a mut String,
    /// This turn's message list. Insert/rerank retrieved context here (e.g. a
    /// RAG service injecting snippets before the latest user message).
    pub history: &'a mut Vec<RigMessage>,
    /// Read-only scope + turn content: `user`, `agent`, `chat`, `task`,
    /// `file_paths`. This is both the partition key (`user.id` scopes all
    /// memory) and what a query-driven service retrieves against.
    pub ctx: &'a InferenceContext,
}

impl<'a> MemoryContext<'a> {
    pub fn new(
        system_prompt: &'a mut String,
        history: &'a mut Vec<RigMessage>,
        ctx: &'a InferenceContext,
    ) -> Self {
        Self {
            system_prompt,
            history,
            ctx,
        }
    }
}

/// The active memory system. One implementation is chosen at boot.
#[async_trait]
pub trait MemoryService: Send + Sync {
    /// Tools this service contributes to the agent. Folded into the builtin
    /// tool set and Cedar-gated like any other tool.
    fn tools(&self) -> Vec<Arc<dyn AgentTool>>;

    /// Per-turn hook, called right before the LLM call with the final prompt
    /// and history in hand. The service mutates [`MemoryContext`] to contribute
    /// whatever it needs this turn: its static usage instructions first (constant
    /// across turns, so they stay in the cacheable prefix), then dynamic blocks
    /// (PKM appends a `<short_memory>` tag; a RAG service could inject
    /// context into `history`). See the caching contract on [`MemoryContext`].
    async fn retrieve(&self, mcx: &mut MemoryContext<'_>) -> Result<(), AppError>;

    /// Register background-maintenance jobs with the scheduler. A registration
    /// lifecycle hook (not a business method): each service registers whatever
    /// periodic upkeep it needs via `scheduler.register_periodic(...)`. Default
    /// no-op (e.g. an event-driven service that maintains itself elsewhere).
    /// Called once at `Scheduler::start()`.
    fn register_maintenance(&self, _scheduler: &crate::scheduler::Scheduler) {}
}
