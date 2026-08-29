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
        let changed = ctx
            .harness
            .chat_service
            .compactor()
            .compact_chat(
                &ctx.user.id,
                &ctx.chat.id,
                &ctx.chat.agent_id,
                &ctx.session.system_prompt,
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
