use async_trait::async_trait;

use crate::chat::message::models::{MessageCommand, MessageRole};
use crate::core::error::AppError;

use super::super::{Command, CommandContext, CommandOutcome};

pub struct TitleCommand;

#[async_trait]
impl Command for TitleCommand {
    fn name(&self) -> &str {
        "title"
    }

    fn description(&self) -> &str {
        "Set or regenerate the chat title."
    }

    fn argument_hint(&self) -> Option<&str> {
        Some("[new title]")
    }

    async fn run(
        &self,
        args: &str,
        ctx: &mut CommandContext<'_>,
    ) -> Result<CommandOutcome, AppError> {
        let trimmed = args.trim();
        let new_title = if !trimmed.is_empty() {
            let req = crate::chat::models::UpdateChatRequest {
                title: Some(trimmed.to_string()),
                space_id: None,
                metadata: None,
            };
            let resp = ctx
                .harness
                .chat_service
                .update_chat(&ctx.user.id, &ctx.chat.id, req)
                .await?;
            resp.title.unwrap_or_else(|| trimmed.to_string())
        } else {
            // Skip command rows (including the `/title` we just persisted) so
            // the title generator doesn't seed off a slash string.
            let seed = ctx
                .harness
                .chat_service
                .get_stored_messages(&ctx.chat.id)
                .await?
                .into_iter()
                .rev()
                .find(|m| {
                    matches!(m.role, MessageRole::User | MessageRole::Contact)
                        && !matches!(m.command, Some(MessageCommand::Command { .. }))
                        && !m.content.is_empty()
                })
                .map(|m| m.content)
                .unwrap_or_default();

            if seed.is_empty() {
                return Ok(CommandOutcome::Message(
                    "Nothing to title yet — send a message first.".to_string(),
                ));
            }

            let agent_id = ctx.chat.agent_id.clone();
            ctx.harness
                .chat_service
                .generate_title(&ctx.chat.id, &agent_id, &seed)
                .await?
        };

        Ok(CommandOutcome::Message(format!(
            "Renamed to '{new_title}'."
        )))
    }
}
