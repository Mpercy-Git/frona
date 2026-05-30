use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::core::state::AppState;
use crate::tool::provider::{
    BUILTIN_PROVIDERS, ToolProvider, ToolProviderKind, ToolProviderStatus, builtin_providers,
    is_configurable_builtin,
};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

#[derive(Serialize, Clone)]
pub struct ToolInfo {
    pub id: String,
    pub description: String,
    pub configurable: bool,
}

#[derive(Serialize, Clone)]
pub struct ToolProviderWithTools {
    #[serde(flatten)]
    pub provider: ToolProvider,
    pub tools: Vec<ToolInfo>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tools", get(list_tools))
        .route("/api/agents/{id}/tools", get(agent_tools))
}

fn cli_provider_id(cli: &crate::tool::cli::CliToolConfig) -> String {
    cli.provider.clone().unwrap_or_else(|| cli.name.clone())
}

fn synthetic_provider_metadata(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "code" => Some(("code", "Execute arbitrary code the agent writes in a supported language.")),
        _ => None,
    }
}

async fn build_catalog(state: &AppState, user_id: &str) -> Vec<ToolProviderWithTools> {
    let mut all_allowed: Vec<String> = BUILTIN_PROVIDERS
        .iter()
        .map(|s| s.id.to_string())
        .collect();
    for cli in state.cli_tools_config.iter() {
        all_allowed.push(cli_provider_id(cli));
        all_allowed.push(cli.name.clone());
    }
    let all_defs = state.tool_manager.definitions(user_id).await;

    let mut by_provider: BTreeMap<String, Vec<ToolInfo>> = BTreeMap::new();
    for def in &all_defs {
        let configurable = is_configurable_builtin(&def.provider_id)
            || state
                .cli_tools_config
                .iter()
                .any(|c| cli_provider_id(c) == def.provider_id);
        by_provider
            .entry(def.provider_id.clone())
            .or_default()
            .push(ToolInfo {
                id: def.id.clone(),
                description: def.description.clone(),
                configurable,
            });
    }

    let mut providers: Vec<ToolProviderWithTools> = builtin_providers()
        .into_iter()
        .map(|p| {
            let tools = by_provider.remove(&p.id).unwrap_or_default();
            ToolProviderWithTools { provider: p, tools }
        })
        .collect();

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for cli in state.cli_tools_config.iter() {
        let provider_id = cli_provider_id(cli);
        if !seen.insert(provider_id.clone()) {
            continue;
        }
        let tools = by_provider.remove(&provider_id).unwrap_or_default();
        let (display_name, description) = match synthetic_provider_metadata(&provider_id) {
            Some((name, desc)) => (name.to_string(), desc.to_string()),
            None => (cli.name.clone(), cli.description.clone()),
        };
        providers.push(ToolProviderWithTools {
            provider: ToolProvider {
                id: provider_id,
                display_name,
                description: Some(description),
                icon: None,
                kind: ToolProviderKind::Builtin,
                status: ToolProviderStatus::Available,
            },
            tools,
        });
    }

    if let Ok(servers) = state.mcp_service.list_for_user(user_id).await {
        for server in servers {
            let status = if state.mcp_manager.is_running(&server.id).await {
                ToolProviderStatus::Available
            } else {
                ToolProviderStatus::Unavailable {
                    reason: format!("Server is {}", server.status),
                }
            };
            let tools: Vec<ToolInfo> = server
                .tool_cache
                .iter()
                .map(|t| ToolInfo {
                    id: format!("mcp__{}__{}", server.handle, t.name),
                    description: t.description.clone(),
                    configurable: true,
                })
                .collect();
            providers.push(ToolProviderWithTools {
                provider: ToolProvider {
                    id: format!("mcp:{}", server.handle),
                    display_name: server.display_name.clone(),
                    description: server.description.clone(),
                    icon: None,
                    kind: ToolProviderKind::Mcp {
                        server_id: server.id.clone(),
                        repository_url: server.repository_url.clone(),
                        version: Some(server.package.version.clone()),
                    },
                    status,
                },
                tools,
            });
        }
    }

    providers
}

async fn list_tools(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Json<Vec<ToolProviderWithTools>> {
    Json(build_catalog(&state, &auth.user_id).await)
}

async fn agent_tools(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ToolInfo>>, ApiError> {
    let agent = state.agent_service.get(&auth.user_id, &id).await
        .map_err(ApiError)?;
    let registry = state.tool_manager.build_agent_registry(&auth.user_id, &agent, &state.policy_service).await;
    let infos: Vec<ToolInfo> = registry
        .definitions()
        .iter()
        .map(|d| ToolInfo {
            id: d.id.clone(),
            description: d.description.clone(),
            configurable: is_configurable_builtin(&d.provider_id)
                || state
                    .cli_tools_config
                    .iter()
                    .any(|c| cli_provider_id(c) == d.provider_id),
        })
        .collect();
    Ok(Json(infos))
}
