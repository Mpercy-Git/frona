use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth::models::{User, UserGroup};
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::policy::models::PolicyAction;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(list_users).post(create_user))
        .route(
            "/api/admin/users/{id}",
            patch(patch_user).delete(delete_user),
        )
        .route(
            "/api/admin/users/{id}/deactivate",
            post(deactivate_user),
        )
        .route(
            "/api/admin/users/{id}/reactivate",
            post(reactivate_user),
        )
        .route("/api/admin/groups", get(list_groups))
}

#[derive(Serialize)]
struct AdminUserListItem {
    id: String,
    handle: crate::core::Handle,
    email: String,
    name: String,
    groups: Vec<String>,
    deactivated_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<User> for AdminUserListItem {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            handle: u.handle,
            email: u.email,
            name: u.name,
            groups: u.groups,
            deactivated_at: u.deactivated_at,
            created_at: u.created_at,
        }
    }
}

#[derive(Serialize)]
struct AdminGroupListItem {
    name: String,
    description: String,
    built_in: bool,
    created_at: DateTime<Utc>,
}

impl From<UserGroup> for AdminGroupListItem {
    fn from(g: UserGroup) -> Self {
        Self {
            name: g.name,
            description: g.description,
            built_in: g.built_in,
            created_at: g.created_at,
        }
    }
}

#[derive(Deserialize)]
struct ListUsersQuery {
    #[serde(default = "default_include_deactivated")]
    include_deactivated: bool,
}

fn default_include_deactivated() -> bool {
    true
}

#[derive(Deserialize)]
struct CreateUserRequest {
    handle: String,
    email: String,
    name: String,
    password: String,
    #[serde(default)]
    groups: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct PatchUserRequest {
    groups: Vec<String>,
}

async fn load_caller(state: &AppState, auth: &AuthUser) -> Result<User, AppError> {
    state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))
}

async fn require(
    state: &AppState,
    caller: &User,
    action: PolicyAction,
) -> Result<(), AppError> {
    let decision = state.policy_service.authorize_user(caller, action).await?;
    if decision.allowed {
        Ok(())
    } else {
        Err(AppError::Forbidden(if decision.diagnostics.is_empty() {
            "Not permitted".into()
        } else {
            decision.diagnostics
        }))
    }
}

async fn list_users(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<Vec<AdminUserListItem>>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require(&state, &caller, PolicyAction::ListUsers).await?;

    let users = state.user_service.list_all(query.include_deactivated).await?;
    Ok(Json(users.into_iter().map(AdminUserListItem::from).collect()))
}

async fn create_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<AdminUserListItem>), ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require(
        &state,
        &caller,
        PolicyAction::ManageUsers {
            target_user_id: "*".into(),
        },
    )
    .await?;

    let groups = req.groups.unwrap_or_default();
    if !groups.is_empty() {
        state
            .user_group_service
            .validate_assignment(&groups)
            .await?;
    }

    let user = state
        .auth_service
        .create_user_with_password(
            &state.user_service,
            crate::auth::models::RegisterRequest {
                handle: req.handle,
                email: req.email,
                name: req.name,
                password: req.password,
            },
            groups,
        )
        .await?;
    state.user_service.ensure_admin_invariant().await?;
    state
        .agent_service
        .clone_all_builtins_for_user(&user.id, &state.storage_service)
        .await?;

    Ok((StatusCode::CREATED, Json(AdminUserListItem::from(user))))
}

async fn patch_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_id): Path<String>,
    Json(req): Json<PatchUserRequest>,
) -> Result<Json<AdminUserListItem>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require(
        &state,
        &caller,
        PolicyAction::ManageUsers {
            target_user_id: target_id.clone(),
        },
    )
    .await?;

    state
        .user_group_service
        .validate_assignment(&req.groups)
        .await?;

    let mut target = state
        .user_service
        .find_by_id(&target_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    target.groups = req.groups;
    target.updated_at = Utc::now();
    let updated = state
        .user_service
        .update(&target)
        .await
        .map_err(translate_invariant_violation)?;
    state.user_service.ensure_admin_invariant().await?;

    Ok(Json(AdminUserListItem::from(updated)))
}

async fn deactivate_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_id): Path<String>,
) -> Result<Json<AdminUserListItem>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require(
        &state,
        &caller,
        PolicyAction::ManageUsers {
            target_user_id: target_id.clone(),
        },
    )
    .await?;

    let updated = state
        .user_service
        .deactivate(&target_id)
        .await
        .map_err(translate_invariant_violation)?;
    let _ = state
        .token_service
        .repo()
        .delete_by_user_id(&target_id)
        .await;
    state.user_service.ensure_admin_invariant().await?;

    Ok(Json(AdminUserListItem::from(updated)))
}

async fn reactivate_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_id): Path<String>,
) -> Result<Json<AdminUserListItem>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require(
        &state,
        &caller,
        PolicyAction::ManageUsers {
            target_user_id: target_id.clone(),
        },
    )
    .await?;

    let updated = state.user_service.reactivate(&target_id).await?;
    Ok(Json(AdminUserListItem::from(updated)))
}

async fn delete_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require(
        &state,
        &caller,
        PolicyAction::ManageUsers {
            target_user_id: target_id.clone(),
        },
    )
    .await?;

    state
        .user_service
        .find_by_id(&target_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    // Tear down running processes and side effects (workspace dirs, vault
    // grants for non-user principals, sandbox policies) before the SurrealDB
    // cascade event deletes the parent rows.
    if let Ok(apps) = state.app_service.list_by_user(&target_id).await {
        for app in apps {
            let _ = state.app_service.destroy(&app.agent_id, &app.id).await;
        }
    }
    if let Ok(servers) = state.mcp_service.list_for_user(&target_id).await {
        for server in servers {
            let _ = state.mcp_service.uninstall(&target_id, &server.id).await;
        }
    }
    if let Ok(channels) = state.channel_service.list_for_user(&target_id).await {
        for channel in channels {
            let _ = state.channel_service.delete(&state, &target_id, &channel.id).await;
        }
    }

    state
        .user_service
        .delete(&target_id)
        .await
        .map_err(translate_invariant_violation)?;

    state.user_service.ensure_admin_invariant().await?;

    Ok(StatusCode::NO_CONTENT)
}

fn translate_invariant_violation(err: AppError) -> AppError {
    let AppError::Database(msg) = &err else {
        return err;
    };
    // SurrealDB embeds the THROW string in the database error message.
    if msg.contains("'last_admin'") || msg.contains("\"last_admin\"") || msg.ends_with("last_admin")
    {
        return AppError::Conflict(json!({ "reason": "last_admin" }).to_string());
    }
    err
}

async fn list_groups(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminGroupListItem>>, ApiError> {
    let caller = load_caller(&state, &auth).await?;
    require(&state, &caller, PolicyAction::ListUsers).await?;

    let groups = state.user_group_service.list_all().await?;
    Ok(Json(groups.into_iter().map(AdminGroupListItem::from).collect()))
}
