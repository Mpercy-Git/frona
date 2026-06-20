//! Server-side `Command` handlers dispatched by `Harness::run_loop` when a
//! user message carries `command: Some(Command { .. })`.
//!
//! Commands are user-initiated and not Cedar-gated; whatever the command's
//! work touches (tools, skills) is still gated downstream.

use std::sync::Arc;

use async_trait::async_trait;

use crate::agent::harness::Harness;
use crate::auth::User;
use crate::chat::models::Chat;
use crate::chat::session::ChatSessionContext;
use crate::chat::message::models::Message;
use crate::core::error::AppError;

pub mod builtin;
pub mod registry;
pub mod render;

pub use registry::CommandRegistry;

pub enum CommandOutcome {
    /// Continue inference; `s` replaces the user-message slot the model sees.
    Prompt(String),
    /// End the turn. `s` is written as an assistant `Message`; downstream
    /// identifies it as a command response by adjacency, not a marker field.
    Message(String),
    /// End the turn silently — no assistant `Message` written.
    End,
}

/// Handler-mutable state. The harness snapshots `chat` + `request` before
/// dispatch and saves whichever rows differ after; `response` is written via
/// the terminal API at end-of-turn. `session` mutations affect this turn only
/// and are never persisted.
pub struct CommandContext<'a> {
    pub harness: &'a Harness,
    pub session: &'a mut ChatSessionContext,
    pub user: &'a User,
    pub chat: &'a mut Chat,
    pub request: &'a mut Message,
    pub response: &'a mut Message,
}

#[async_trait]
pub trait Command: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn argument_hint(&self) -> Option<&str> {
        None
    }

    async fn run(
        &self,
        args: &str,
        ctx: &mut CommandContext<'_>,
    ) -> Result<CommandOutcome, AppError>;
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommandManifest {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

impl CommandManifest {
    pub fn from_command(cmd: &Arc<dyn Command>) -> Self {
        Self {
            name: cmd.name().to_string(),
            description: cmd.description().to_string(),
            argument_hint: cmd.argument_hint().map(|s| s.to_string()),
        }
    }
}
