use async_trait::async_trait;

use crate::core::error::AppError;

use super::super::{Command, CommandContext, CommandOutcome};

pub struct ClearCommand;

#[async_trait]
impl Command for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }

    fn description(&self) -> &str {
        "Delete all messages in this chat."
    }

    async fn run(
        &self,
        _args: &str,
        ctx: &mut CommandContext<'_>,
    ) -> Result<CommandOutcome, AppError> {
        ctx.harness
            .chat_service
            .delete_messages_for_chat(&ctx.user.id, &ctx.chat.id)
            .await?;
        Ok(CommandOutcome::End)
    }
}
