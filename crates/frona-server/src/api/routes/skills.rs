use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::agent::skill::service::{
    RepoBrowseResult, SkillListItem, SkillPreview, SkillSearchResult, UpdateCheckResult,
};
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
}

async fn list_installed(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<SkillListItem>>, ApiError> {
    let items = state.skill_service.list_installed()?;
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
    let agent_pair = match req.agent_id.as_deref() {
        Some(agent_id) => {
            let agent = state.agent_service.get(&auth.user_id, agent_id).await?;
            Some((auth.handle.clone(), agent.handle))
        }
        None => None,
    };
    let agent_ref = agent_pair.as_ref().map(|(u, h)| (u, h));
    let items = state.skill_service.install_batch(&req.repo, &req.skill_names, agent_ref).await?;
    Ok(Json(items))
}

#[derive(Deserialize)]
struct UninstallQuery {
    agent_id: Option<String>,
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
    } else {
        state.skill_service.uninstall(&name).await?;
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
