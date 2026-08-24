use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use rig_core::client::{ModelListingClient, Nothing};
use rig_core::model::ModelList;
use rig_core::providers::{
    anthropic, deepseek, gemini, groq, mira, mistral, moonshot, ollama, openai, openrouter,
};
use serde::Serialize;

use crate::core::state::AppState;
use crate::inference::metadata::ModelCatalogSnapshot;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/api/config/providers/{id}/models",
        get(list_provider_models),
    )
}

#[derive(Debug, Clone, Serialize)]
struct ModelInfo {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
}

#[derive(serde::Deserialize)]
struct ListModelsQuery {
    api_key: Option<String>,
    base_url: Option<String>,
}

fn validation(message: impl Into<String>) -> ApiError {
    ApiError(crate::core::error::AppError::Validation(message.into()))
}

fn internal(message: impl Into<String>) -> ApiError {
    ApiError(crate::core::error::AppError::Internal(message.into()))
}

fn require_api_key(provider: &str, api_key: Option<&str>) -> Result<String, ApiError> {
    api_key
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| validation(format!("API key required for provider '{provider}'")))
}

fn openai_compatible_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "cohere" => Some("https://api.cohere.ai"),
        "perplexity" => Some("https://api.perplexity.ai"),
        "together" => Some("https://api.together.xyz"),
        "xai" => Some("https://api.x.ai"),
        "hyperbolic" => Some("https://api.hyperbolic.xyz"),
        _ => None,
    }
}

macro_rules! list_with_api_key {
    ($provider:expr, $module:ident, $key:expr, $base_url:expr) => {{
        let client: $module::Client = if let Some(url) = $base_url {
            $module::Client::builder()
                .api_key($key)
                .base_url(url)
                .build()
                .map_err(|error| internal(format!("Failed to configure {}: {error}", $provider)))?
        } else {
            $module::Client::builder()
                .api_key($key)
                .build()
                .map_err(|error| internal(format!("Failed to configure {}: {error}", $provider)))?
        };
        client.list_models().await
    }};
}

async fn list_models(
    provider: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<ModelList, ApiError> {
    let result = match provider {
        "ollama" => {
            let client: ollama::Client = ollama::Client::builder()
                .api_key(Nothing)
                .base_url(base_url.unwrap_or("http://localhost:11434"))
                .build()
                .map_err(|error| internal(format!("Failed to configure ollama: {error}")))?;
            client.list_models().await
        }
        provider => {
            let key = require_api_key(provider, api_key)?;
            match provider {
                "openai" => list_with_api_key!(provider, openai, &key, base_url),
                "anthropic" => list_with_api_key!(provider, anthropic, &key, base_url),
                "groq" => list_with_api_key!(provider, groq, &key, base_url),
                "openrouter" => list_with_api_key!(provider, openrouter, &key, base_url),
                "deepseek" => list_with_api_key!(provider, deepseek, &key, base_url),
                "gemini" => list_with_api_key!(provider, gemini, &key, base_url),
                "mistral" => list_with_api_key!(provider, mistral, &key, base_url),
                "moonshot" => list_with_api_key!(provider, moonshot, &key, base_url),
                "mira" => list_with_api_key!(provider, mira, &key, base_url),
                provider if openai_compatible_base_url(provider).is_some() => {
                    let client = openai::CompletionsClient::builder()
                        .api_key(&key)
                        .base_url(
                            base_url
                                .or_else(|| openai_compatible_base_url(provider))
                                .expect("guard ensures a default base URL"),
                        )
                        .build()
                        .map_err(|error| {
                            internal(format!("Failed to configure {provider}: {error}"))
                        })?;
                    client.list_models().await
                }
                _ => {
                    return Err(validation(format!(
                        "Model listing not supported for provider '{provider}'"
                    )));
                }
            }
        }
    };

    result.map_err(|error| {
        internal(format!(
            "Provider '{provider}' failed to list models: {error}"
        ))
    })
}

fn model_info(
    model: rig_core::model::Model,
    provider: &str,
    catalog: &ModelCatalogSnapshot,
) -> ModelInfo {
    let mut context_window = model.context_length.map(u64::from);
    let mut max_tokens = model.max_output_tokens.map(u64::from);

    if (context_window.is_none() || max_tokens.is_none())
        && let Some(entry) = catalog.lookup_prefix(provider, &model.id)
    {
        if context_window.is_none() && entry.limit.context > 0 {
            context_window = Some(entry.limit.context);
        }
        if max_tokens.is_none() && entry.limit.output > 0 {
            max_tokens = Some(entry.limit.output);
        }
    }

    ModelInfo {
        id: model.id,
        name: model.name,
        context_window,
        max_tokens,
    }
}

async fn list_provider_models(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<ListModelsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let saved = state.config.providers.get(&provider_id);
    let api_key = query
        .api_key
        .or_else(|| saved.and_then(|provider| provider.api_key.clone()));
    let base_url = query
        .base_url
        .or_else(|| saved.and_then(|provider| provider.base_url.clone()));

    let catalog = state.model_catalog.current();
    let models = list_models(&provider_id, api_key.as_deref(), base_url.as_deref())
        .await?
        .data
        .into_iter()
        .map(|model| model_info(model, &provider_id, &catalog))
        .collect::<Vec<_>>();

    Ok(Json(serde_json::json!({ "models": models })))
}
