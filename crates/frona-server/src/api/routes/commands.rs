//! `GET /api/chats/{chat_id}/commands` — discovery endpoint feeding the
//! composer's `/` and `@` autocomplete dropdowns.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::chat::command::CommandManifest;
use crate::core::error::AppError;
use crate::core::state::AppState;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/chats/{chat_id}/commands", get(list_commands))
}

#[derive(Debug, Serialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// `false` when SKILL.md sets `disable-model-invocation: true`.
    pub model_invocable: bool,
}

#[derive(Debug, Serialize)]
pub struct CommandsResponse {
    pub skills: Vec<SkillEntry>,
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Serialize)]
pub struct CommandEntry {
    /// Wire-format handle (for agents: lowercase, no spaces).
    pub name: String,
    /// Pretty name. Defaults to `name` for static commands and skills.
    pub display_name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

impl From<CommandManifest> for CommandEntry {
    fn from(m: CommandManifest) -> Self {
        Self {
            display_name: m.name.clone(),
            name: m.name,
            description: m.description,
            argument_hint: m.argument_hint,
        }
    }
}

async fn list_commands(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<String>,
) -> Result<Json<CommandsResponse>, ApiError> {
    let chat = state
        .chat_service
        .get_chat(&auth.user_id, &chat_id)
        .await
        .map_err(ApiError::from)?;

    let agent = state
        .agent_service
        .find_by_id(&chat.agent_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(AppError::NotFound(format!("agent {}", chat.agent_id))))?;

    let user = state
        .user_service
        .find_by_id(&auth.user_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(AppError::NotFound(format!("user {}", auth.user_id))))?;

    let mut skills: Vec<SkillEntry> = state
        .skill_service
        .list(&user.handle, &agent.handle, agent.skills.as_deref())
        .await
        .into_iter()
        .map(|s| SkillEntry {
            name: s.name,
            description: s.description,
            argument_hint: s.argument_hint,
            model_invocable: !s.disable_model_invocation,
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));

    // Precedence: static commands > skills > agents.
    let static_manifests = state
        .harness
        .commands
        .list_static()
        .into_iter()
        .map(|c| CommandManifest::from_command(&c))
        .collect::<Vec<_>>();

    let mut taken: std::collections::HashSet<String> = static_manifests
        .iter()
        .map(|m| m.name.clone())
        .collect();
    for s in &skills {
        taken.insert(s.name.clone());
    }

    let agents = state
        .agent_service
        .list(&auth.user_id)
        .await
        .map_err(ApiError::from)?;
    let agent_entries: Vec<CommandEntry> = agents
        .into_iter()
        .filter(|a| !taken.contains(a.handle.as_str()))
        .map(|a| CommandEntry {
            display_name: a.name.clone(),
            name: a.handle.to_string(),
            description: a
                .identity
                .get("description")
                .cloned()
                .unwrap_or_else(|| format!("Delegate this message to {}.", a.name)),
            argument_hint: Some("[prompt]".to_string()),
        })
        .collect();

    let mut commands: Vec<CommandEntry> = static_manifests.into_iter().map(Into::into).collect();
    commands.extend(agent_entries);
    commands.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(CommandsResponse { skills, commands }))
}
