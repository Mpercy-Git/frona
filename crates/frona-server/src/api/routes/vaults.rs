use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::error::ApiError;
use crate::api::middleware::auth::AuthUser;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::credential::vault::models::*;
use crate::credential::vault::provider::create_vault_provider;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/vaults", post(create_connection).get(list_connections))
        .route("/api/vaults/approve", post(approve_request))
        .route("/api/vaults/deny", post(deny_request))
        .route("/api/vaults/grants", get(list_grants).post(create_grant))
        .route("/api/vaults/grants/{id}", delete(revoke_grant))
        .route(
            "/api/vaults/local/items",
            get(list_local_items).post(create_local_item),
        )
        .route(
            "/api/vaults/local/items/{id}",
            axum::routing::put(update_local_item).delete(delete_local_item),
        )
        .route("/api/vaults/test", post(test_vault))
        .route("/api/vaults/{id}", delete(delete_connection))
        .route("/api/vaults/{id}/toggle", post(toggle_connection))
        .route("/api/vaults/{id}/test", post(test_connection))
        .route("/api/vaults/{id}/items", get(search_items).post(search_items_inline))
        .route("/api/vaults/{id}/items/{item_id}/fields", get(item_fields))
}

async fn create_connection(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateVaultConnectionRequest>,
) -> Result<Json<VaultConnectionResponse>, ApiError> {
    let response = state
        .vault_service
        .create_connection(&auth.user_id, req)
        .await?;
    Ok(Json(response))
}

async fn list_connections(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<VaultConnectionResponse>>, ApiError> {
    let connections = state
        .vault_service
        .list_connections(&auth.user_id)
        .await?;
    Ok(Json(connections))
}

async fn delete_connection(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .vault_service
        .delete_connection(&auth.user_id, &id)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

async fn toggle_connection(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ToggleVaultConnectionRequest>,
) -> Result<Json<VaultConnectionResponse>, ApiError> {
    let response = state
        .vault_service
        .toggle_connection(&auth.user_id, &id, req.enabled)
        .await?;
    Ok(Json(response))
}

async fn test_connection(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .vault_service
        .test_connection(&auth.user_id, &id)
        .await?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

fn default_max_results() -> usize {
    10
}

async fn search_items(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<VaultItem>>, ApiError> {
    let items = state
        .vault_service
        .search_items(&auth.user_id, &id, &query.q, query.max_results)
        .await?;
    Ok(Json(items))
}

#[derive(Deserialize)]
struct InlineSearchRequest {
    provider: VaultProviderType,
    config: VaultConnectionConfig,
    #[serde(default)]
    q: String,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

async fn search_items_inline(
    _auth: AuthUser,
    Path(_id): Path<String>,
    Json(req): Json<InlineSearchRequest>,
) -> Result<Json<Vec<VaultItem>>, ApiError> {
    let tmp = tempfile::tempdir()
        .map_err(|e| ApiError::from(AppError::Tool(format!("Failed to create temp dir: {e}"))))?;
    let provider = create_vault_provider(req.provider, req.config, tmp.path().to_path_buf())?;
    let items = provider.search(&req.q, req.max_results).await?;
    Ok(Json(items))
}


async fn approve_request(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ApproveVaultRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let chat = state
        .chat_service
        .get_chat(&auth.user_id, &req.chat_id)
        .await
        .map_err(ApiError::from)?;

    let secret = state
        .vault_service
        .get_secret(&auth.user_id, &req.connection_id, &req.vault_item_id)
        .await?;

    let pending_te = state.chat_service
        .find_pending_tool_call(&req.chat_id)
        .await
        .map_err(ApiError::from)?;

    let (original_query, original_reason) = pending_te.as_ref()
        .and_then(|te| te.tool_data.as_ref())
        .map(|td| match td {
            crate::inference::tool_call::MessageTool::VaultApproval { query, reason, .. } => {
                (query.clone(), reason.clone())
            }
            _ => (String::new(), String::new()),
        })
        .unwrap_or_default();

    {
        use crate::core::Principal;
        let principal = Principal::agent(chat.agent_id.clone());

        let target =
            binding_target_for_approval(req.env_var_prefix.as_deref(), &original_query);
        let (scope, expires_at) = binding_scope_for_duration(&req.grant_duration, &req.chat_id);

        if !matches!(req.grant_duration, GrantDuration::Once) {
            state
                .vault_service
                .create_grant(
                    &auth.user_id,
                    principal.clone(),
                    &req.connection_id,
                    &req.vault_item_id,
                    &original_query,
                    &req.grant_duration,
                )
                .await?;
        }

        state
            .vault_service
            .create_binding(
                &auth.user_id,
                principal.clone(),
                &original_query,
                &req.connection_id,
                &req.vault_item_id,
                target,
                scope,
                expires_at,
            )
            .await?;

        state
            .vault_service
            .log_access(
                &auth.user_id,
                principal,
                &req.chat_id,
                &req.connection_id,
                &req.vault_item_id,
                req.env_var_prefix.as_deref(),
                &original_query,
                &original_reason,
            )
            .await?;
    }

    let result_text = if let Some(ref prefix) = req.env_var_prefix {
        let env_vars = secret.to_env_vars(prefix);
        let var_names: Vec<String> = env_vars.iter().map(|(k, _)| k.clone()).collect();
        format!(
            "Credentials loaded into environment variables: {}. Use these in CLI commands.",
            var_names.join(", ")
        )
    } else {
        let mut parts = Vec::new();
        parts.push(format!("Credentials for: {}", secret.name));
        if let Some(ref u) = secret.username {
            parts.push(format!("Username: {u}"));
        }
        if let Some(ref p) = secret.password {
            parts.push(format!("Password: {p}"));
        }
        for (k, v) in &secret.fields {
            parts.push(format!("{k}: {v}"));
        }
        parts.join("\n")
    };

    if let Some(te) = pending_te {
        let message_id = te.message_id.clone();
        let resolved = state
            .chat_service
            .resolve_tool_call(&te.id, Some(result_text.clone()))
            .await
            .map_err(ApiError::from)?
            .into_message();

        state.broadcast_service.send(crate::chat::broadcast::BroadcastEvent {
            user_id: auth.user_id.clone(),
            chat_id: Some(req.chat_id.clone()),
            space_id: None,
            kind: crate::chat::broadcast::BroadcastEventKind::ToolResolved { message: resolved },
        });

        let still_pending = state.chat_service
            .has_pending_tools_for_message(&message_id).await.unwrap_or(false);
        if !still_pending {
            let user_id = auth.user_id.clone();
            let chat_id = req.chat_id.clone();
            let state_clone = state.clone();
            tokio::spawn(async move {
                crate::agent::task::executor::resume_or_notify(&state_clone, &user_id, &chat_id, &message_id).await;
            });
        }
    }

    Ok(Json(serde_json::json!({ "approved": true })))
}

async fn deny_request(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<DenyVaultRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .chat_service
        .get_chat(&auth.user_id, &req.chat_id)
        .await
        .map_err(ApiError::from)?;

    let pending_te = state.chat_service
        .find_pending_tool_call(&req.chat_id)
        .await
        .map_err(ApiError::from)?;

    if let Some(te) = pending_te {
        let message_id = te.message_id.clone();
        let denied = state
            .chat_service
            .deny_tool_call(
                &te.id,
                Some("User denied the credential request.".to_string()),
            )
            .await
            .map_err(ApiError::from)?
            .into_message();

        state.broadcast_service.send(crate::chat::broadcast::BroadcastEvent {
            user_id: auth.user_id.clone(),
            chat_id: Some(req.chat_id.clone()),
            space_id: None,
            kind: crate::chat::broadcast::BroadcastEventKind::ToolResolved { message: denied },
        });

        let still_pending = state.chat_service
            .has_pending_tools_for_message(&message_id).await.unwrap_or(false);
        if !still_pending {
            let user_id = auth.user_id.clone();
            let chat_id = req.chat_id.clone();
            let state_clone = state.clone();
            tokio::spawn(async move {
                crate::agent::task::executor::resume_or_notify(&state_clone, &user_id, &chat_id, &message_id).await;
            });
        }
    }

    Ok(Json(serde_json::json!({ "denied": true })))
}

async fn list_grants(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<VaultGrantResponse>>, ApiError> {
    let grants = state.vault_service.list_grants(&auth.user_id).await?;
    Ok(Json(grants))
}

async fn revoke_grant(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .vault_service
        .revoke_grant(&auth.user_id, &id)
        .await?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

async fn create_grant(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateGrantRequest>,
) -> Result<Json<VaultGrantResponse>, ApiError> {
    let grant = state
        .vault_service
        .create_grant(
            &auth.user_id,
            req.principal.clone(),
            &req.connection_id,
            &req.vault_item_id,
            &req.query,
            &GrantDuration::Permanent,
        )
        .await?;

    state
        .vault_service
        .create_binding(
            &auth.user_id,
            req.principal,
            &req.query,
            &req.connection_id,
            &req.vault_item_id,
            req.target,
            BindingScope::Durable,
            None,
        )
        .await?;

    Ok(Json(grant.into()))
}

async fn item_fields(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((connection_id, item_id)): Path<(String, String)>,
) -> Result<Json<Vec<String>>, ApiError> {
    let secret = state
        .vault_service
        .get_secret(&auth.user_id, &connection_id, &item_id)
        .await?;
    let mut fields = Vec::new();
    if secret.username.is_some() {
        fields.push("USERNAME".to_string());
    }
    if secret.password.is_some() {
        fields.push("PASSWORD".to_string());
    }
    for key in secret.fields.keys() {
        fields.push(key.to_uppercase().replace(' ', "_"));
    }
    Ok(Json(fields))
}

// --- Inline test route ---

#[derive(Deserialize)]
struct TestVaultRequest {
    provider: VaultProviderType,
    config: VaultConnectionConfig,
}

async fn test_vault(
    _auth: AuthUser,
    Json(req): Json<TestVaultRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tmp = tempfile::tempdir()
        .map_err(|e| ApiError::from(AppError::Tool(format!("Failed to create temp dir: {e}"))))?;
    let provider = create_vault_provider(req.provider, req.config, tmp.path().to_path_buf())?;
    provider.test_connection().await?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// --- Local item routes ---

async fn create_local_item(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateLocalItemRequest>,
) -> Result<Json<CredentialResponse>, ApiError> {
    let response = state
        .vault_service
        .create_credential(&auth.user_id, req)
        .await?;
    Ok(Json(response))
}

async fn list_local_items(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<CredentialResponse>>, ApiError> {
    let credentials = state.vault_service.list_credentials(&auth.user_id).await?;
    Ok(Json(credentials))
}

async fn update_local_item(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateLocalItemRequest>,
) -> Result<Json<CredentialResponse>, ApiError> {
    let response = state
        .vault_service
        .update_credential(&auth.user_id, &id, req)
        .await?;
    Ok(Json(response))
}

async fn delete_local_item(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .vault_service
        .delete_credential(&auth.user_id, &id)
        .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

fn binding_target_for_approval(
    requested_prefix: Option<&str>,
    fallback_query: &str,
) -> crate::credential::vault::models::CredentialTarget {
    use crate::credential::vault::models::CredentialTarget;
    CredentialTarget::Prefix {
        env_var_prefix: requested_prefix
            .map(|p| p.to_string())
            .unwrap_or_else(|| fallback_query.to_string()),
    }
}

fn binding_scope_for_duration(
    duration: &GrantDuration,
    chat_id: &str,
) -> (
    crate::credential::vault::models::BindingScope,
    Option<chrono::DateTime<chrono::Utc>>,
) {
    use crate::credential::vault::models::BindingScope;
    match duration {
        GrantDuration::Once => (
            BindingScope::Chat {
                chat_id: chat_id.to_string(),
            },
            None,
        ),
        GrantDuration::Hours(h) => (
            BindingScope::Durable,
            Some(chrono::Utc::now() + chrono::Duration::hours(*h as i64)),
        ),
        GrantDuration::Days(d) => (
            BindingScope::Durable,
            Some(chrono::Utc::now() + chrono::Duration::days(*d as i64)),
        ),
        GrantDuration::Permanent => (BindingScope::Durable, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::vault::models::{BindingScope, CredentialTarget};

    #[test]
    fn binding_target_uses_requested_prefix_when_supplied() {
        let target = binding_target_for_approval(Some("GH"), "github");
        match target {
            CredentialTarget::Prefix { env_var_prefix } => assert_eq!(env_var_prefix, "GH"),
            _ => panic!("expected Prefix target"),
        }
    }

    #[test]
    fn binding_target_falls_back_to_query_when_no_prefix() {
        let target = binding_target_for_approval(None, "github");
        match target {
            CredentialTarget::Prefix { env_var_prefix } => assert_eq!(env_var_prefix, "github"),
            _ => panic!("expected Prefix target"),
        }
    }

    #[test]
    fn once_duration_produces_chat_scoped_binding_with_no_expiry() {
        let (scope, expires_at) = binding_scope_for_duration(&GrantDuration::Once, "chat-xyz");
        match scope {
            BindingScope::Chat { chat_id } => assert_eq!(chat_id, "chat-xyz"),
            _ => panic!("Once should produce Chat scope"),
        }
        assert!(expires_at.is_none());
    }

    #[test]
    fn hours_duration_produces_durable_with_expiry() {
        let (scope, expires_at) = binding_scope_for_duration(&GrantDuration::Hours(2), "chat1");
        assert!(matches!(scope, BindingScope::Durable));
        let expiry = expires_at.expect("Hours should set expires_at");
        let delta = expiry - chrono::Utc::now();
        assert!(delta > chrono::Duration::minutes(119));
        assert!(delta < chrono::Duration::minutes(121));
    }

    #[test]
    fn days_duration_produces_durable_with_expiry() {
        let (scope, expires_at) = binding_scope_for_duration(&GrantDuration::Days(7), "chat1");
        assert!(matches!(scope, BindingScope::Durable));
        let expiry = expires_at.expect("Days should set expires_at");
        let delta = expiry - chrono::Utc::now();
        assert!(delta > chrono::Duration::days(6) + chrono::Duration::hours(23));
        assert!(delta < chrono::Duration::days(7) + chrono::Duration::hours(1));
    }

    #[test]
    fn permanent_duration_produces_durable_with_no_expiry() {
        let (scope, expires_at) =
            binding_scope_for_duration(&GrantDuration::Permanent, "chat1");
        assert!(matches!(scope, BindingScope::Durable));
        assert!(expires_at.is_none());
    }
}
