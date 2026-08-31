use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
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

fn default_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("https://api.openai.com/v1"),
        "anthropic" => Some("https://api.anthropic.com"),
        "groq" => Some("https://api.groq.com/openai/v1"),
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "deepseek" => Some("https://api.deepseek.com"),
        "gemini" => Some("https://generativelanguage.googleapis.com"),
        "cohere" => Some("https://api.cohere.ai"),
        "mistral" => Some("https://api.mistral.ai"),
        "perplexity" => Some("https://api.perplexity.ai"),
        "together" => Some("https://api.together.xyz"),
        "xai" => Some("https://api.x.ai"),
        "hyperbolic" => Some("https://api.hyperbolic.xyz"),
        "moonshot" => Some("https://api.moonshot.cn/v1"),
        "mira" => Some("https://api.mira.network"),
        "galadriel" => Some("https://api.galadriel.com/v1/verified"),
        "huggingface" => Some("https://router.huggingface.co"),
        "ollama" => Some("http://localhost:11434"),
        "zai" => Some("https://api.z.ai/api/paas/v4"),
        "venice" => Some("https://api.venice.ai/api/v1"),
        "minimax" => Some("https://api.minimax.io/v1"),
        "llamafile" => Some("http://localhost:8080"),
        // `generic` has no default host on purpose - the configured base_url
        // is the only thing that identifies the endpoint.
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum ResponseFormat {
    OpenAi,
    OpenRouter,
    Gemini,
    Ollama,
}

fn models_endpoint(provider: &str) -> Option<(&'static str, ResponseFormat)> {
    match provider {
        "openrouter" => Some(("/models", ResponseFormat::OpenRouter)),
        "openai" | "groq" | "cohere" | "mistral" | "together" | "xai" | "hyperbolic"
        | "moonshot" | "deepseek" | "zai" | "venice" | "minimax" | "llamafile" | "generic" => {
            Some(("/models", ResponseFormat::OpenAi))
        }
        "anthropic" => Some(("/v1/models", ResponseFormat::OpenAi)),
        "perplexity" => Some(("/models", ResponseFormat::OpenAi)),
        "gemini" => Some(("/v1beta/models", ResponseFormat::Gemini)),
        "ollama" => Some(("/api/tags", ResponseFormat::Ollama)),
        _ => None,
    }
}

fn extract_models(
    body: &serde_json::Value,
    format: ResponseFormat,
    provider: &str,
    catalog: &ModelCatalogSnapshot,
) -> Vec<ModelInfo> {
    let mut models = match format {
        ResponseFormat::OpenAi => extract_openai(body),
        ResponseFormat::OpenRouter => extract_openrouter(body),
        ResponseFormat::Gemini => extract_gemini(body),
        ResponseFormat::Ollama => extract_ollama(body),
    };

    for model in &mut models {
        if (model.context_window.is_none() || model.max_tokens.is_none())
            && let Some(entry) = catalog.lookup_prefix(provider, &model.id)
        {
            if model.context_window.is_none() && entry.limit.context > 0 {
                model.context_window = Some(entry.limit.context);
            }
            if model.max_tokens.is_none() && entry.limit.output > 0 {
                model.max_tokens = Some(entry.limit.output);
            }
        }
    }

    models
}

fn extract_openai(body: &serde_json::Value) -> Vec<ModelInfo> {
    body.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?.to_string();
                    // Anthropic uses "display_name", others may not have one
                    let name = m
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    Some(ModelInfo {
                        id,
                        name,
                        context_window: None,
                        max_tokens: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_openrouter(body: &serde_json::Value) -> Vec<ModelInfo> {
    body.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?.to_string();
                    let name = m.get("name").and_then(|v| v.as_str()).map(String::from);
                    let context_window = m
                        .get("context_length")
                        .and_then(|v| v.as_u64());
                    let max_tokens = m
                        .get("top_provider")
                        .and_then(|tp| tp.get("max_completion_tokens"))
                        .and_then(|v| v.as_u64());
                    Some(ModelInfo {
                        id,
                        name,
                        context_window,
                        max_tokens,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_gemini(body: &serde_json::Value) -> Vec<ModelInfo> {
    body.get("models")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let raw_name = m.get("name")?.as_str()?;
                    let id = raw_name.strip_prefix("models/").unwrap_or(raw_name).to_string();
                    let name = m
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let context_window = m.get("inputTokenLimit").and_then(|v| v.as_u64());
                    let max_tokens = m.get("outputTokenLimit").and_then(|v| v.as_u64());
                    Some(ModelInfo {
                        id,
                        name,
                        context_window,
                        max_tokens,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_ollama(body: &serde_json::Value) -> Vec<ModelInfo> {
    body.get("models")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("name")?.as_str()?.to_string();
                    Some(ModelInfo {
                        id,
                        name: None,
                        context_window: None,
                        max_tokens: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(serde::Deserialize)]
struct ListModelsQuery {
    api_key: Option<String>,
    base_url: Option<String>,
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
        .or_else(|| saved.and_then(|p| p.api_key.clone()));
    let base_url = query
        .base_url
        .or_else(|| saved.and_then(|p| p.base_url.clone()));

    let (endpoint_path, format) = models_endpoint(&provider_id).ok_or_else(|| {
        ApiError(crate::core::error::AppError::Validation(format!(
            "Model listing not supported for provider '{provider_id}'"
        )))
    })?;

    let base = base_url
        .as_deref()
        .or_else(|| default_base_url(&provider_id))
        .ok_or_else(|| {
            ApiError(crate::core::error::AppError::Validation(format!(
                "No base URL for provider '{provider_id}'"
            )))
        })?;

    let url = format!("{}{}", base.trim_end_matches('/'), endpoint_path);

    let mut req = state.http_client.get(&url);

    if let Some(ref key) = api_key
        && !key.is_empty()
    {
        if provider_id == "anthropic" {
            req = req.header("x-api-key", key);
            req = req.header("anthropic-version", "2023-06-01");
        } else if provider_id == "gemini" {
            // Gemini's native API (generativelanguage.googleapis.com) does NOT
            // accept `Authorization: Bearer <key>` — it expects the key in the
            // `x-goog-api-key` header. Bearer auth returns 401 UNAUTHENTICATED
            // (API_KEY_SERVICE_BLOCKED), including for the new AQ. auth keys.
            req = req.header("x-goog-api-key", key);
        } else {
            req = req.bearer_auth(key);
        }
    }

    let resp = req
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| {
            ApiError(crate::core::error::AppError::Internal(format!(
                "Failed to fetch models from {provider_id} ({url}): {e:?}"
            )))
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ApiError(crate::core::error::AppError::Internal(format!(
            "Provider '{provider_id}' returned {status}: {body}"
        ))));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| {
        ApiError(crate::core::error::AppError::Internal(format!(
            "Failed to parse models response from {provider_id}: {e}"
        )))
    })?;

    let catalog = state.model_catalog.current();
    let models = extract_models(&body, format, &provider_id, &catalog);

    Ok(Json(serde_json::json!({ "models": models })))
}
