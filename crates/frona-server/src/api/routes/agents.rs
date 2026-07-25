use std::collections::HashMap;
use std::path::Path as StdPath;

use axum::extract::{Multipart, Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use futures::stream::{self, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use crate::agent::config::parse_frontmatter;
use crate::agent::models::{
    Agent, AgentResponse, CreateAgentRequest, Model, UpdateAgentRequest,
};
use crate::chat::broadcast::{BroadcastEvent, BroadcastEventKind};
use crate::inference::tool_loop::InferenceEventKind;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::error::AppError;
use crate::core::state::AppState;

/// How many agents to build responses for at once when listing. Bounded so a
/// user with many agents doesn't fan out an unbounded burst of policy
/// evaluations and file reads at the database.
const AGENT_LIST_CONCURRENCY: usize = 8;

/// Resolve `model_group` → live `ModelEntry`. Returns `None` when the group
/// name doesn't match a configured group (config drift, agent referencing a
/// removed group), in which case `AgentResponse.model` stays `None`.
fn resolve_model(state: &AppState, model_group_name: &str) -> Option<Model> {
    let group = state
        .chat_service
        .provider_registry()
        .resolve_model_group(model_group_name)
        .ok()?;
    let entry = state.model_catalog.current().lookup(&group.main).cloned();
    Some(Model {
        provider: group.main.provider.clone(),
        model_id: group.main.model_id.clone(),
        context_window: group.context_window,
        entry,
    })
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/agents", get(list_agents).post(create_agent))
        .route(
            "/api/agents/{id}",
            get(get_agent).put(update_agent).delete(delete_agent),
        )
        .route("/api/agents/{id}/skills", get(list_agent_skills))
        .route("/api/agents/{id}/avatar", put(upload_avatar))
        .route(
            "/api/agents/{id}/shares",
            get(list_agent_shares).post(share_agent),
        )
        .route(
            "/api/agents/{id}/shares/{recipient_id}",
            put(update_agent_share).delete(unshare_agent),
        )
}

async fn validate_request_sandbox_paths(
    state: &AppState,
    auth: &AuthUser,
    policy: Option<&crate::policy::sandbox::SandboxPolicy>,
) -> Result<(), AppError> {
    let Some(policy) = policy else {
        return Ok(());
    };
    let owned_agents: std::collections::HashSet<String> = state
        .agent_service
        .list(&auth.user_id)
        .await?
        .into_iter()
        .map(|a| a.id)
        .collect();
    policy.validate_paths(&auth.handle, |id| owned_agents.contains(id))
}

async fn sync_agent_tools(
    state: &AppState,
    user_id: &str,
    user_handle: &crate::core::Handle,
    agent_handle: &crate::core::Handle,
    selected_tools: &[String],
) -> Result<(), crate::core::error::AppError> {
    state
        .policy_service
        .reconcile_agent_tools(user_id, user_handle, agent_handle, selected_tools)
        .await
        .map(|_| ())
        .map_err(crate::core::error::AppError::from)
}

fn resolve_default_prompt(state: &AppState, user_handle: &crate::core::Handle, agent_handle: &crate::core::Handle) -> String {
    state
        .storage_service
        .agent_workspace(user_handle, agent_handle)
        .read("AGENT.md")
        .map(|c| parse_frontmatter(&c).template)
        .unwrap_or_default()
}

/// Build the API view of an agent for `requesting_user_id`.
///
/// Definition-scoped fields (tools, sandbox policy, default prompt, avatar) are
/// always resolved under the **agent owner's** identity, so a shared agent is
/// shown exactly as its owner configured it. When the requester isn't the
/// owner, `shared_by`/`read_only` are set for the UI.
async fn to_response(
    state: &AppState,
    requesting_user_id: &str,
    agent: Agent,
) -> Result<AgentResponse, AppError> {
    let owner_id = agent.user_id.clone();
    let owner_handle = state.user_service.handle_of(&owner_id).await?;
    let is_shared = owner_id != requesting_user_id;

    let registry = state
        .tool_manager
        .build_agent_registry(&owner_id, &agent, &state.policy_service, None)
        .await;
    let tools: Vec<String> = registry.definitions().iter().map(|d| d.id.clone()).collect();
    let sandbox_policy = state
        .policy_service
        .evaluate_sandbox_policy(
            crate::policy::service::SandboxPrincipalRef::agent(
                &owner_id,
                &owner_handle,
                &agent.handle,
            ),
            false,
        )
        .await?
        .as_ref()
        .clone();
    let agent_id = agent.id.clone();
    let agent_handle = agent.handle.clone();
    let mut response = AgentResponse::from_agent(agent, tools, sandbox_policy);
    response.model = resolve_model(state, &response.model_group);
    response.default_prompt = resolve_default_prompt(state, &owner_handle, &agent_handle);
    if is_shared {
        response.is_shared = true;
        response.shared_by = Some(owner_handle.to_string());
    }
    if let Some(value) = response.identity.get("avatar")
        && !value.is_empty()
    {
        response.avatar_url = if value.starts_with("http://") || value.starts_with("https://") {
            Some(value.clone())
        } else if !value.contains('/') {
            state
                .presign_service
                .sign_with_expiry_by_user_id(
                    &format!("agent:{agent_id}"),
                    value,
                    &owner_id,
                    crate::credential::presign::PresignService::LONG_TERM_EXPIRY_SECS,
                )
                .await
                .ok()
        } else {
            None
        };
    }
    Ok(response)
}

async fn create_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    let tools = req.tools.clone();
    validate_request_sandbox_paths(&state, &auth, req.sandbox_policy.as_ref()).await?;
    let agent = state.agent_service.create(&auth.user_id, req).await?;

    if let Some(tool_list) = tools {
        sync_agent_tools(&state, &auth.user_id, &auth.handle, &agent.handle, &tool_list).await?;
        state.policy_service.invalidate_all_caches();
    }

    let response = to_response(&state, &auth.user_id, agent).await?;
    Ok(Json(response))
}

async fn list_agents(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentResponse>>, ApiError> {
    let agents = state.agent_service.list(&auth.user_id).await?;

    let count_map: HashMap<String, u64> = state
        .db
        .query("SELECT agent_id, count() AS count FROM chat WHERE user_id = $user_id GROUP BY agent_id")
        .bind(("user_id", auth.user_id.clone()))
        .await
        .and_then(|mut r| r.take::<Vec<serde_json::Value>>(0))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            let agent_id = v.get("agent_id")?.as_str()?.to_string();
            let count = v.get("count")?.as_u64()?;
            Some((agent_id, count))
        })
        .collect();

    // Agents shared with this user (use-only, read-only view). Backing agents
    // that have since been deleted are skipped.
    let shares = state
        .agent_share_service
        .list_shared_with(&auth.user_id)
        .await?;
    let shared_agents: Vec<Agent> = stream::iter(shares)
        .map(|share| {
            let state = &state;
            async move { state.agent_service.find_by_id(&share.agent_id).await }
        })
        .buffered(AGENT_LIST_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect();

    // `to_response` is expensive per agent — it builds the tool registry,
    // evaluates the sandbox policy and reads the default prompt from disk.
    // Run a bounded number concurrently instead of strictly one at a time;
    // `buffered` preserves order, so owned agents still precede shared ones.
    let mut responses: Vec<AgentResponse> = stream::iter(agents.into_iter().chain(shared_agents))
        .map(|agent| {
            let state = &state;
            let user_id = auth.user_id.as_str();
            async move { to_response(state, user_id, agent).await }
        })
        .buffered(AGENT_LIST_CONCURRENCY)
        .try_collect()
        .await?;

    for response in &mut responses {
        if let Some(&count) = count_map.get(response.id.as_str()) {
            response.chat_count = count;
        }
    }

    Ok(Json(responses))
}

async fn get_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    // Owner or shared-recipient may view; `get_accessible` returns Forbidden
    // otherwise. Editing endpoints stay owner-only.
    let (agent, _access) = state.agent_service.get_accessible(&auth.user_id, &id).await?;
    let response = to_response(&state, &auth.user_id, agent).await?;
    Ok(Json(response))
}

async fn update_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    let tools = req.tools.clone();
    validate_request_sandbox_paths(&state, &auth, req.sandbox_policy.as_ref()).await?;
    let agent = state.agent_service.update(&auth.user_id, &id, req).await?;

    if let Some(tool_list) = tools {
        sync_agent_tools(&state, &auth.user_id, &auth.handle, &agent.handle, &tool_list).await?;
        // Force-invalidate all caches synchronously — moka's per-key
        // invalidate() is eventually-consistent and the stale decision
        // cache entries can survive long enough for to_response() below
        // to read them, causing disabled tools to reappear as enabled.
        state.policy_service.invalidate_all_caches();
    }

    let response = to_response(&state, &auth.user_id, agent).await?;

    state.broadcast_service.send(BroadcastEvent {
        user_id: auth.user_id,
        chat_id: None,
        space_id: None,
        kind: BroadcastEventKind::Inference(InferenceEventKind::EntityUpdated {
            table: "agent".to_string(),
            record_id: id,
            fields: serde_json::to_value(&response).unwrap_or_default(),
        }),
    });

    Ok(Json(response))
}

async fn delete_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), ApiError> {
    let agent = state.agent_service.get(&auth.user_id, &id).await?;
    state.agent_service.delete(&auth.user_id, &id).await?;
    state
        .policy_service
        .delete_agent_policies(&auth.user_id, &auth.handle, &agent.handle)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Agent sharing (use-only, per recipient)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ShareAgentRequest {
    /// Recipient username/handle or email.
    recipient: String,
    /// Opt-in: let the recipient's runs use this agent's owner-granted
    /// credentials. Defaults to false.
    #[serde(default)]
    delegate_credentials: bool,
}

#[derive(Deserialize)]
struct UpdateShareRequest {
    delegate_credentials: bool,
}

#[derive(Serialize)]
struct AgentShareResponse {
    recipient_id: String,
    recipient_handle: String,
    recipient_name: String,
    /// Access level ("use" today).
    level: String,
    /// Whether the recipient's runs may use the owner's agent credentials.
    delegate_credentials: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn build_share_responses(
    state: &AppState,
    shares: Vec<crate::agent::share::models::AgentShare>,
) -> Vec<AgentShareResponse> {
    let mut out = Vec::with_capacity(shares.len());
    for s in shares {
        // Resolve recipient display info; fall back to the id if the user row
        // has since vanished.
        let (handle, name) = match state.user_service.find_by_id(&s.recipient_id).await {
            Ok(Some(u)) => (u.handle.to_string(), u.name),
            _ => (s.recipient_id.clone(), String::new()),
        };
        out.push(AgentShareResponse {
            recipient_id: s.recipient_id,
            recipient_handle: handle,
            recipient_name: name,
            level: "use".to_string(),
            delegate_credentials: s.delegate_credentials,
            created_at: s.created_at,
        });
    }
    out
}

/// `GET /api/agents/{id}/shares` — who this agent is shared with (owner only).
async fn list_agent_shares(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<AgentShareResponse>>, ApiError> {
    // Owner-only: `get` returns Forbidden for non-owners.
    let _ = state.agent_service.get(&auth.user_id, &id).await?;
    let shares = state.agent_share_service.list_for_agent(&id).await?;
    Ok(Json(build_share_responses(&state, shares).await))
}

/// `POST /api/agents/{id}/shares` — grant a user use-only access (owner only).
async fn share_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ShareAgentRequest>,
) -> Result<Json<Vec<AgentShareResponse>>, ApiError> {
    let _ = state.agent_service.get(&auth.user_id, &id).await?;
    let share = state
        .agent_share_service
        .share(&auth.user_id, &id, &req.recipient)
        .await?;
    if req.delegate_credentials {
        state
            .agent_share_service
            .set_delegation(&id, &share.recipient_id, true)
            .await?;
    }
    let shares = state.agent_share_service.list_for_agent(&id).await?;
    Ok(Json(build_share_responses(&state, shares).await))
}

/// `PUT /api/agents/{id}/shares/{recipient_id}` — toggle credential delegation
/// for an existing share (owner only).
async fn update_agent_share(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, recipient_id)): Path<(String, String)>,
    Json(req): Json<UpdateShareRequest>,
) -> Result<Json<Vec<AgentShareResponse>>, ApiError> {
    let _ = state.agent_service.get(&auth.user_id, &id).await?;
    state
        .agent_share_service
        .set_delegation(&id, &recipient_id, req.delegate_credentials)
        .await?;
    let shares = state.agent_share_service.list_for_agent(&id).await?;
    Ok(Json(build_share_responses(&state, shares).await))
}

/// `DELETE /api/agents/{id}/shares/{recipient_id}` — revoke access (owner only).
async fn unshare_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, recipient_id)): Path<(String, String)>,
) -> Result<Json<Vec<AgentShareResponse>>, ApiError> {
    let _ = state.agent_service.get(&auth.user_id, &id).await?;
    state.agent_share_service.unshare(&id, &recipient_id).await?;
    let shares = state.agent_share_service.list_for_agent(&id).await?;
    Ok(Json(build_share_responses(&state, shares).await))
}

async fn list_agent_skills(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::agent::skill::service::SkillListItem>>, ApiError> {
    // Viewable by owner or shared-recipient; skills resolve under the owner.
    let (agent, _access) = state.agent_service.get_accessible(&auth.user_id, &id).await?;
    let owner_handle = state.user_service.handle_of(&agent.user_id).await?;
    let skills = state.skill_service.list(&owner_handle, &agent.handle, None).await;
    let items = skills.into_iter().map(|s| crate::agent::skill::service::SkillListItem {
        name: s.name,
        description: s.description,
        source: None,
        installed_at: None,
        scope: s.scope,
    }).collect();
    Ok(Json(items))
}

const MAX_AVATAR_SIZE: usize = 2 * 1024 * 1024;

async fn upload_avatar(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let agent = state.agent_service.get(&auth.user_id, &id).await?;

    let mut file_data: Option<(String, Vec<u8>)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError(AppError::Validation(e.to_string())))?
    {
        if field.name() == Some("file") {
            let filename = field.file_name().unwrap_or("avatar").to_string();
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError(AppError::Validation(e.to_string())))?;
            if bytes.len() > MAX_AVATAR_SIZE {
                return Err(ApiError(AppError::Validation(
                    "Avatar too large (max 2MB)".into(),
                )));
            }
            file_data = Some((filename, bytes.to_vec()));
        }
    }

    let (filename, bytes) = file_data
        .ok_or_else(|| ApiError(AppError::Validation("Missing file field".into())))?;

    let ext = StdPath::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg");
    let avatar_filename = format!("avatar.{ext}");

    let workspace = state.storage_service.agent_workspace(&auth.handle, &agent.handle);
    workspace
        .write_bytes(&avatar_filename, &bytes)
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    let owner = format!("agent:{id}");
    let presigned_url = state
        .presign_service
        .sign_with_expiry_by_user_id(
            &owner,
            &avatar_filename,
            &auth.user_id,
            crate::credential::presign::PresignService::LONG_TERM_EXPIRY_SECS,
        )
        .await?;

    Ok(Json(serde_json::json!({
        "filename": avatar_filename,
        "url": presigned_url,
    })))
}

