use async_trait::async_trait;

use crate::core::error::AppError;

use super::super::{Command, CommandContext, CommandOutcome};

const DEFAULT_MAX_OUTPUT_TOKENS: usize = 4096;

pub struct CompactCommand;

#[async_trait]
impl Command for CompactCommand {
    fn name(&self) -> &str {
        "compact"
    }

    fn description(&self) -> &str {
        "Compress older messages into a summary to free up context."
    }

    async fn run(
        &self,
        _args: &str,
        ctx: &mut CommandContext<'_>,
    ) -> Result<CommandOutcome, AppError> {
        // Same weighing as a normal turn: what the builder replays around each
        // message is mostly its tool calls, so `/compact` has to see them too or
        // it reports "already at optimal size" on a chat that is over the window.
        let tool_calls = ctx
            .harness
            .chat_service
            .get_tool_calls(&ctx.chat.id)
            .await
            .unwrap_or_default();
        let changed = ctx
            .harness
            .chat_service
            .compactor()
            .compact_chat(
                &ctx.user.id,
                &ctx.chat.id,
                &ctx.chat.agent_id,
                &ctx.session.system_prompt,
                &tool_calls,
                ctx.session.model_group.context_window,
                DEFAULT_MAX_OUTPUT_TOKENS,
            )
            .await?
            .compacted;
        let status = if changed {
            "Compacted older messages into a summary."
        } else {
            "Chat is already at optimal size — no compaction needed."
        };
        Ok(CommandOutcome::Message(status.to_string()))
    }
}
