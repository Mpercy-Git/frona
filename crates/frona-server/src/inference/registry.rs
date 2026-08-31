use std::collections::HashMap;
use std::sync::Arc;

use rig_core::client::Nothing;
use rig_core::providers::{
    anthropic, azure, cohere, deepseek, gemini, groq, huggingface, hyperbolic, llamafile, minimax,
    mira, mistral, moonshot, ollama, openai, openrouter, perplexity, together, venice, xai, zai,
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

    /// Resolve a named utility model group, falling back to `primary` when the
    /// named group isn't configured. This is the override mechanism shared by
    /// background utilities (title, compaction, …): define a model group with
    /// the utility's name to override; otherwise the utility uses `primary`.
    pub fn utility_model_group(&self, name: &str) -> Result<ModelGroup, InferenceError> {
        match self.get_model_group(name) {
            Ok(g) => Ok(g.clone()),
            Err(_) => self.get_model_group("primary").cloned(),
        }
    }

    /// Iterate every configured model group. Order is unspecified — callers that
    /// need determinism should sort.
    pub fn iter_model_groups(&self) -> impl Iterator<Item = &ModelGroup> {
        self.model_groups.values()
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
        // Not via init_builder_provider! because Anthropic needs a request hook
        // (prompt caching) attached to the provider.
        "anthropic" => {
            let key = require_api_key(name, entry)?;
            let client: anthropic::Client = if let Some(url) = &entry.base_url {
                anthropic::Client::builder()
                    .api_key(&key)
                    .base_url(url)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            } else {
                anthropic::Client::builder()
                    .api_key(&key)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            };
            Ok(Arc::new(
                RigProvider::new(client, counter.clone()).with_hook(hooks::anthropic),
            ) as Arc<dyn ModelProvider>)
        }
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
        "groq" => {
            let key = require_api_key(name, entry)?;
            let client: groq::Client = if let Some(url) = &entry.base_url {
                groq::Client::builder()
                    .api_key(&key)
                    .base_url(url)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("groq: {e}")))?
            } else {
                groq::Client::new(&key)
                    .map_err(|e| InferenceError::ConfigError(format!("groq: {e}")))?
            };
            Ok(Arc::new(
                RigProvider::new(client, counter.clone()).with_hook(hooks::groq),
            ) as Arc<dyn ModelProvider>)
        }
        // Not via init_api_key_provider! because OpenRouter needs both a
        // request hook (rename `provider_routing` -> `provider`) and a model
        // decorator (explicit prompt-caching breakpoint).
        "openrouter" => {
            let key = require_api_key(name, entry)?;
            let client: openrouter::Client = if let Some(url) = &entry.base_url {
                openrouter::Client::builder()
                    .api_key(&key)
                    .base_url(url)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            } else {
                openrouter::Client::new(&key)
                    .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?
            };
            Ok(Arc::new(
                RigProvider::new(client, counter.clone())
                    .with_hook(hooks::openrouter)
                    .with_model_decorator(openrouter_prompt_caching),
            ) as Arc<dyn ModelProvider>)
        }
        // Azure keys its data plane off a per-resource endpoint plus an API
        // version in the query string, so neither `Client::new` nor the shared
        // builder macros fit. The model id is the *deployment* name, which the
        // caller supplies as usual.
        "azure" => {
            let key = require_api_key(name, entry)?;
            let endpoint = entry.base_url.clone().ok_or_else(|| {
                InferenceError::ConfigError(
                    "Provider 'azure' requires a base_url — your resource endpoint, \
                     e.g. https://<resource>.openai.azure.com"
                        .to_string(),
                )
            })?;
            // `impl From<S: Into<String>> for AzureOpenAIAuth` yields a bearer
            // token, which is the Entra ID path. A configured `api_key` means
            // the resource key, so name the variant explicitly.
            let mut builder = azure::Client::builder()
                .api_key(azure::AzureOpenAIAuth::ApiKey(key))
                .azure_endpoint(endpoint);
            if let Some(version) = &entry.api_version {
                builder = builder.api_version(version);
            }
            let client = builder
                .build()
                .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?;
            Ok(Arc::new(
                RigProvider::new(client, counter.clone()).with_hook(hooks::openai),
            ) as Arc<dyn ModelProvider>)
        }
        "deepseek" => init_api_key_provider!(name, entry, deepseek, counter),
        "zai" => init_api_key_provider!(name, entry, zai, counter),
        "venice" => init_api_key_provider!(name, entry, venice, counter),
        "minimax" => init_api_key_provider!(name, entry, minimax, counter),
        // Local server, like Ollama: no API key, and the base_url is the whole
        // configuration.
        "llamafile" => {
            let client: llamafile::Client = if let Some(url) = &entry.base_url {
                llamafile::Client::builder()
                    .api_key(Nothing)
                    .base_url(url)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("llamafile: {e}")))?
            } else {
                llamafile::Client::builder()
                    .api_key(Nothing)
                    .build()
                    .map_err(|e| InferenceError::ConfigError(format!("llamafile: {e}")))?
            };
            Ok(Arc::new(RigProvider::new(client, counter.clone())) as Arc<dyn ModelProvider>)
        }
        // Any OpenAI-compatible chat-completions endpoint. Deliberately *not*
        // `hooks::openai`: that hook rewrites `max_tokens` into
        // `max_completion_tokens` for gpt-5/o-series, and servers like vLLM,
        // LM Studio and llama.cpp reject the rewritten field. `base_url` is
        // required — there is no sensible default host for "generic" — while
        // `api_key` is optional, since local servers commonly ignore auth.
        "generic" => {
            let url = entry.base_url.as_deref().ok_or_else(|| {
                InferenceError::ConfigError(
                    "Provider 'generic' requires a base_url pointing at an \
                     OpenAI-compatible endpoint"
                        .to_string(),
                )
            })?;
            let client = openai::CompletionsClient::builder()
                .api_key(entry.api_key.as_deref().unwrap_or(""))
                .base_url(url)
                .build()
                .map_err(|e| InferenceError::ConfigError(format!("{name}: {e}")))?;
            Ok(Arc::new(RigProvider::new(client, counter.clone())) as Arc<dyn ModelProvider>)
        }
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

/// Put an explicit `cache_control` breakpoint on the system prompt unless the
/// model group opts out.
///
/// frona sends the same large system prompt and tool definitions on every turn
/// of a tool loop, so on providers that honour explicit breakpoints the prefix
/// is billed once as a cache write and then at the (much cheaper) cache-read
/// rate for the rest of the loop. Providers that cache automatically ignore the
/// marker, so defaulting it on costs nothing there. Opting out is worth it only
/// for a genuinely one-shot workload, where the write is never read back.
fn openrouter_prompt_caching(
    model: rig_core::providers::openrouter::CompletionModel,
    model_ref: &ModelRef,
) -> rig_core::providers::openrouter::CompletionModel {
    let enabled = match &model_ref.provider {
        ProviderModel::OpenRouter { params } => params.prompt_caching.unwrap_or(true),
        _ => true,
    };
    if enabled {
        model.with_prompt_caching()
    } else {
        model
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

    fn azure_entry(base_url: Option<&str>, api_version: Option<&str>) -> ModelProviderConfig {
        ModelProviderConfig {
            api_key: Some("test-key".to_string()),
            base_url: base_url.map(str::to_string),
            api_version: api_version.map(str::to_string),
            enabled: true,
        }
    }

    fn provider_entry(api_key: Option<&str>, base_url: Option<&str>) -> ModelProviderConfig {
        ModelProviderConfig {
            api_key: api_key.map(str::to_string),
            base_url: base_url.map(str::to_string),
            enabled: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn azure_initialises_from_an_endpoint_and_key() {
        let counter = InferenceCounter::new(BroadcastService::new());
        for version in [None, Some("2025-01-01-preview")] {
            assert!(
                init_provider(
                    "azure",
                    &azure_entry(Some("https://example.openai.azure.com"), version),
                    &counter,
                )
                .is_ok(),
                "api_version {version:?} should build",
            );
        }
    }

    /// Azure has no shared host — every resource has its own endpoint — so a
    /// missing base_url can't fall back to anything and must fail at startup.
    #[tokio::test]
    async fn azure_without_an_endpoint_is_a_config_error() {
        let counter = InferenceCounter::new(BroadcastService::new());
        // `expect_err` would need the Ok side to be Debug; `Arc<dyn ModelProvider>`
        // isn't, so match instead.
        let Err(err) = init_provider("azure", &azure_entry(None, None), &counter) else {
            panic!("azure without a base_url must not initialise");
        };
        assert!(
            err.to_string().contains("base_url"),
            "the error should name what's missing, got: {err}"
        );
    }

    /// `provider: generic` parsed fine long before it could be initialised —
    /// `init_provider` had no arm for it, and `from_config` only *warns* on a
    /// failed init, so the provider silently didn't exist and the failure
    /// surfaced much later as `ProviderNotConfigured`.
    // `BroadcastService::new` spawns a task, so these need a runtime.
    #[tokio::test]
    async fn generic_provider_initialises_from_a_base_url_alone() {
        let counter = InferenceCounter::new(BroadcastService::new());
        assert!(
            init_provider(
                "generic",
                &provider_entry(None, Some("http://localhost:8000/v1")),
                &counter,
            )
            .is_ok(),
            "a local OpenAI-compatible server needs no API key"
        );
    }

    /// There is no sensible default host for "generic", so a missing base_url
    /// has to fail loudly at startup rather than resolve to some other API.
    // `BroadcastService::new` spawns a task, so these need a runtime.
    #[tokio::test]
    async fn generic_provider_without_a_base_url_is_a_config_error() {
        let counter = InferenceCounter::new(BroadcastService::new());
        // `expect_err` would need the Ok side to be Debug; `Arc<dyn ModelProvider>`
        // isn't, so match instead.
        let Err(err) = init_provider("generic", &provider_entry(Some("k"), None), &counter) else {
            panic!("generic without a base_url must not initialise");
        };
        assert!(
            err.to_string().contains("base_url"),
            "the error should name what's missing, got: {err}"
        );
    }

    // `BroadcastService::new` spawns a task, so these need a runtime.
    #[tokio::test]
    async fn newly_wired_providers_initialise() {
        let counter = InferenceCounter::new(BroadcastService::new());
        for name in ["zai", "venice", "minimax"] {
            assert!(
                init_provider(name, &provider_entry(Some("test-key"), None), &counter).is_ok(),
                "{name} should initialise from an API key alone"
            );
        }
        assert!(
            init_provider("llamafile", &provider_entry(None, None), &counter).is_ok(),
            "llamafile defaults to its local base URL"
        );
    }

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
