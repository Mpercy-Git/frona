use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::agent::skill::service::{
    RepoBrowseResult, SkillListItem, SkillPreview, SkillSearchResult, UpdateCheckResult,
};
use crate::auth::models::ADMINS_GROUP;
use crate::core::error::AppError;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/skills", get(list_installed))
        .route("/api/skills/search", get(search_skills))
        .route("/api/skills/browse", get(browse_repo))
        .route("/api/skills/preview", get(preview_skill))
        .route("/api/skills/install", post(install_skill))
        .route("/api/skills/check", get(check_updates))
        .route("/api/skills/{name}", delete(uninstall_skill))
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum InstallScope {
    #[default]
    User,
    Shared,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ListScope {
    #[default]
    User,
    Shared,
    Builtin,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Deserialize)]
struct BrowseQuery {
    repo: String,
}

#[derive(Deserialize)]
struct PreviewQuery {
    repo: String,
    name: String,
}

#[derive(Deserialize)]
struct InstallRequest {
    repo: String,
    skill_names: Vec<String>,
    agent_id: Option<String>,
    #[serde(default)]
    scope: InstallScope,
}

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    scope: ListScope,
}

async fn require_admin(state: &AppState, auth: &AuthUser) -> Result<(), ApiError> {
    let user = state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| ApiError(AppError::NotFound("User not found".into())))?;
    if !user.groups.iter().any(|g| g == ADMINS_GROUP) {
        return Err(ApiError(AppError::Forbidden(
            "Server-wide skill management requires admin privileges".into(),
        )));
    }
    Ok(())
}

async fn list_installed(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<Json<Vec<SkillListItem>>, ApiError> {
    let items = match params.scope {
        ListScope::User => state.skill_service.list_installed_for_user(&auth.handle)?,
        ListScope::Shared => state.skill_service.list_installed()?,
        ListScope::Builtin => state.skill_service.list_builtin()?,
    };
    Ok(Json(items))
}

async fn search_skills(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<SkillSearchResult>>, ApiError> {
    let results = state.skill_service.search(&params.q).await?;
    Ok(Json(results))
}

async fn browse_repo(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<BrowseQuery>,
) -> Result<Json<RepoBrowseResult>, ApiError> {
    let result = state.skill_service.get_skills(&params.repo).await?;
    Ok(Json(result))
}

async fn preview_skill(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<PreviewQuery>,
) -> Result<Json<SkillPreview>, ApiError> {
    let preview = state.skill_service.preview(&params.repo, &params.name).await?;
    Ok(Json(preview))
}

async fn install_skill(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<InstallRequest>,
) -> Result<Json<Vec<SkillListItem>>, ApiError> {
    if let Some(agent_id) = req.agent_id.as_deref() {
        let agent = state.agent_service.get(&auth.user_id, agent_id).await?;
        let agent_ref = (&auth.handle, &agent.handle);
        let items = state
            .skill_service
            .install_batch(&req.repo, &req.skill_names, Some(agent_ref))
            .await?;
        return Ok(Json(items));
    }

    match req.scope {
        InstallScope::User => {
            let items = state
                .skill_service
                .install_batch_for_user(&auth.handle, &req.repo, &req.skill_names)
                .await?;
            Ok(Json(items))
        }
        InstallScope::Shared => {
            require_admin(&state, &auth).await?;
            let items = state
                .skill_service
                .install_batch(&req.repo, &req.skill_names, None)
                .await?;
            Ok(Json(items))
        }
    }
}

#[derive(Deserialize)]
struct UninstallQuery {
    agent_id: Option<String>,
    #[serde(default)]
    scope: InstallScope,
}

async fn uninstall_skill(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<UninstallQuery>,
) -> Result<Json<()>, ApiError> {
    if let Some(agent_id) = &params.agent_id {
        let agent = state.agent_service.get(&auth.user_id, agent_id).await?;
        state.skill_service.uninstall_agent_skill(&auth.handle, &agent.handle, &name).await?;
        return Ok(Json(()));
    }
    match params.scope {
        InstallScope::User => {
            state.skill_service.uninstall_for_user(&auth.handle, &name).await?;
        }
        InstallScope::Shared => {
            require_admin(&state, &auth).await?;
            state.skill_service.uninstall(&name).await?;
        }
    }
    Ok(Json(()))
}

async fn check_updates(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<UpdateCheckResult>>, ApiError> {
    let results = state.skill_service.check_updates().await?;
    Ok(Json(results))
}
