use std::collections::HashMap;
use std::sync::Arc;

use rig_core::client::Nothing;
use rig_core::providers::{
    anthropic, cohere, deepseek, gemini, groq, huggingface, hyperbolic, mira, mistral, moonshot,
    ollama, openai, openrouter, perplexity, together, xai,
};

use super::config::{
    InferenceConfig, ModelGroup, ModelProviderConfig, ModelRegistryConfig, RetryConfig,
};
use super::error::InferenceError;
use super::hooks;
use super::provider::{InferenceCounter, ModelProvider, ModelRef, OpenAiProvider, RigProvider};
use crate::chat::broadcast::BroadcastService;
use crate::core::config::ProviderModel;

#[derive(Clone)]
pub struct ModelProviderRegistry {
    providers: Arc<HashMap<String, Arc<dyn ModelProvider>>>,
    model_groups: Arc<HashMap<String, ModelGroup>>,
    protocol_defaults: Arc<HashMap<String, crate::core::config::OpenAiApi>>,
    inference: InferenceConfig,
}

impl ModelProviderRegistry {
    pub fn from_config(
        config: ModelRegistryConfig,
        broadcast: BroadcastService,
        inference: &InferenceConfig,
        catalog: &crate::inference::metadata::ModelCatalogSnapshot,
    ) -> Result<Self, InferenceError> {
        let model_groups = config.parse_model_groups(inference, catalog)?;
        let mut providers: HashMap<String, Arc<dyn ModelProvider>> = HashMap::new();
        let counter = InferenceCounter::new(broadcast);

        for (name, entry) in &config.providers {
            if !entry.enabled {
                tracing::info!(provider = %name, "Provider disabled, skipping");
                continue;
            }

            match init_provider(name, entry, &counter) {
                Ok(provider) => {
                    tracing::info!(provider = %name, "Provider initialized");
                    providers.insert(name.clone(), provider);
                }
                Err(e) => {
                    tracing::warn!(provider = %name, error = %e, "Failed to initialize provider");
                }
            }
        }

        if providers.is_empty() {
            tracing::warn!(
                "No inference providers configured — chat will fail until a provider is available"
            );
        }

        Ok(Self {
            providers: Arc::new(providers),
            model_groups: Arc::new(model_groups),
            protocol_defaults: Arc::new(catalog.protocol_defaults.clone()),
            inference: inference.clone(),
        })
    }

    pub fn get_provider(&self, name: &str) -> Result<&dyn ModelProvider, InferenceError> {
        self.providers
            .get(name)
            .map(|p| p.as_ref())
            .ok_or_else(|| InferenceError::ProviderNotConfigured(name.to_string()))
    }

    pub fn get_model_group(&self, group_name: &str) -> Result<&ModelGroup, InferenceError> {
        self.model_groups
            .get(group_name)
            .ok_or_else(|| InferenceError::ModelGroupNotFound(group_name.to_string()))
    }

    pub fn resolve_model_group(&self, name_or_ref: &str) -> Result<ModelGroup, InferenceError> {
        if name_or_ref.contains('/') {
            let mut model_ref = ModelRef::parse(name_or_ref)?;
            let protocol_key = model_ref.as_str();
            if let ProviderModel::OpenAI { api, .. } = &mut model_ref.provider {
                *api = Some(
                    self.protocol_defaults
                        .get(&protocol_key)
                        .copied()
                        .unwrap_or_default(),
                );
            }
            Ok(ModelGroup {
                name: name_or_ref.to_string(),
                main: model_ref,
                fallbacks: vec![],
                max_tokens: Some(self.inference.default_max_tokens),
                temperature: None,
                // Ad-hoc model_ref (e.g. from a slash command). No catalog
                // lookup at this layer - fall back to the conservative
                // default. Callers that want a precise window should configure
                // a proper ModelGroup.
                context_window: crate::inference::context::DEFAULT_CONTEXT_WINDOW,
                retry: RetryConfig::default(),
                inference: self.inference.clone(),
            })
        } else {
            match self.get_model_group(name_or_ref) {
                Ok(g) => Ok(g.clone()),
                Err(_) => self.get_model_group("primary").cloned(),
            }
        }
    }

    pub fn has_model_group(&self, group_name: &str) -> bool {
        self.model_groups.contains_key(group_name)
    }

    pub fn for_testing(
        providers: HashMap<String, Arc<dyn ModelProvider>>,
        model_groups: HashMap<String, ModelGroup>,
    ) -> Self {
        Self {
            providers: Arc::new(providers),
            model_groups: Arc::new(model_groups),
            protocol_defaults: Arc::new(HashMap::new()),
            inference: InferenceConfig::default(),
        }
    }
}

macro_rules! init_api_key_provider {
    ($name:expr, $entry:expr, $mod:ident, $counter:expr) => {{
        let key = require_api_key($name, $entry)?;
        let client: $mod::Client = if let Some(url) = &$entry.base_url {
            $mod::Client::builder()
                .api_key(&key)
                .base_url(url)
                .build()
                .map_err(|e| InferenceError::ConfigError(format!("{}: {e}", $name)))?
        } else {
            $mod::Client::new(&key)
                .map_err(|e| InferenceError::ConfigError(format!("{}: {e}", $name)))?
        };
        Ok(Arc::new(RigProvider::new(client, $counter.clone())) as Arc<dyn ModelProvider>)
    }};
}

macro_rules! init_builder_provider {
    ($name:expr, $entry:expr, $mod:ident, $counter:expr) => {{
        let key = require_api_key($name, $entry)?;
        let client: $mod::Client = if let Some(url) = &$entry.base_url {
            $mod::Client::builder()
                .api_key(&key)
                .base_url(url)
                .build()
                .map_err(|e| InferenceError::ConfigError(format!("{}: {e}", $name)))?
        } else {
            $mod::Client::builder()
                .api_key(&key)
                .build()
                .map_err(|e| InferenceError::ConfigError(format!("{}: {e}", $name)))?
        };
        Ok(Arc::new(RigProvider::new(client, $counter.clone())) as Arc<dyn ModelProvider>)
    }};
}

fn init_provider(
    name: &str,
    entry: &ModelProviderConfig,
    counter: &InferenceCounter,
) -> Result<Arc<dyn ModelProvider>, InferenceError> {
    match name {
        "openai" => {
            let key = require_api_key(name, entry)?;
            let chat_completions: openai::CompletionsClient = if let Some(url) = &entry.base_url {
                openai::CompletionsClient::builder()
                    .api_key(&key)
                    .base_url(url)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            } else {
                openai::CompletionsClient::new(&key)
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            };
            let responses: openai::Client = if let Some(url) = &entry.base_url {
                openai::Client::builder()
                    .api_key(&key)
                    .base_url(url)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            } else {
                openai::Client::new(&key)
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            };
            Ok(Arc::new(OpenAiProvider::new(
                chat_completions,
                responses,
                counter.clone(),
            )) as Arc<dyn ModelProvider>)
        }
        "anthropic" => init_builder_provider!(name, entry, anthropic, counter),
        "ollama" => {
            let client: ollama::Client = if let Some(url) = &entry.base_url {
                ollama::Client::builder()
                    .api_key(Nothing)
                    .base_url(url)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("ollama: {e}")))?
            } else {
                ollama::Client::new(Nothing)
                    .map_err(|e| InferenceError::ConfigError(format!("ollama: {e}")))?
            };
            Ok(Arc::new(
                RigProvider::new(client, counter.clone()).with_hook(hooks::ollama),
            ))
        }
        "groq" => init_api_key_provider!(name, entry, groq, counter),
        "openrouter" => init_api_key_provider!(name, entry, openrouter, counter),
        "deepseek" => init_api_key_provider!(name, entry, deepseek, counter),
        "gemini" => init_api_key_provider!(name, entry, gemini, counter),
        "cohere" => init_api_key_provider!(name, entry, cohere, counter),
        "mistral" => init_api_key_provider!(name, entry, mistral, counter),
        "perplexity" => init_api_key_provider!(name, entry, perplexity, counter),
        "together" => init_api_key_provider!(name, entry, together, counter),
        "xai" => init_api_key_provider!(name, entry, xai, counter),
        "hyperbolic" => init_api_key_provider!(name, entry, hyperbolic, counter),
        "moonshot" => init_api_key_provider!(name, entry, moonshot, counter),
        "mira" => init_api_key_provider!(name, entry, mira, counter),
        // Rig 0.41 removed its dedicated Galadriel adapter. Galadriel exposes an
        // OpenAI-compatible chat-completions endpoint, so keep the configured provider
        // working through Rig's OpenAI client instead of dropping support entirely.
        "galadriel" => {
            let key = require_api_key(name, entry)?;
            let url = entry
                .base_url
                .as_deref()
                .unwrap_or("https://api.galadriel.com/v1/verified");
            let client = openai::CompletionsClient::builder()
                .api_key(&key)
                .base_url(url)
                .build()
                .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?;
            Ok(
                Arc::new(RigProvider::new(client, counter.clone()).with_hook(hooks::openai))
                    as Arc<dyn ModelProvider>,
            )
        }
        "huggingface" => init_api_key_provider!(name, entry, huggingface, counter),
        _ => Err(InferenceError::ProviderNotConfigured(format!(
            "Unknown provider: {name}"
        ))),
    }
}

fn require_api_key(provider: &str, entry: &ModelProviderConfig) -> Result<String, InferenceError> {
    entry.api_key.clone().ok_or_else(|| {
        InferenceError::ConfigError(format!(
            "Provider '{provider}' requires an api_key but none was provided"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::OpenAiApi;

    fn registry_with_protocol_defaults(
        protocol_defaults: HashMap<String, OpenAiApi>,
    ) -> ModelProviderRegistry {
        ModelProviderRegistry {
            providers: Arc::new(HashMap::new()),
            model_groups: Arc::new(HashMap::new()),
            protocol_defaults: Arc::new(protocol_defaults),
            inference: InferenceConfig::default(),
        }
    }

    #[test]
    fn ad_hoc_openai_model_uses_catalog_protocol_default() {
        let mut defaults = HashMap::new();
        defaults.insert("openai/gpt-test".to_string(), OpenAiApi::Responses);
        let group = registry_with_protocol_defaults(defaults)
            .resolve_model_group("openai/gpt-test")
            .unwrap();
        let ProviderModel::OpenAI { api, .. } = group.main.provider else {
            panic!("expected openai provider");
        };
        assert_eq!(api, Some(OpenAiApi::Responses));
    }

    #[test]
    fn ad_hoc_openai_model_without_metadata_uses_chat_completions() {
        let group = registry_with_protocol_defaults(HashMap::new())
            .resolve_model_group("openai/unknown")
            .unwrap();
        let ProviderModel::OpenAI { api, .. } = group.main.provider else {
            panic!("expected openai provider");
        };
        assert_eq!(api, Some(OpenAiApi::ChatCompletions));
    }
}
