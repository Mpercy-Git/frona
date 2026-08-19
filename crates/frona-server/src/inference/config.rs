use std::collections::HashMap;

pub use crate::core::config::{
    CommonModelFields, InferenceConfig, ModelGroupConfig, ModelProviderConfig, ProviderModel,
    RetryConfig,
};

use super::error::InferenceError;
use super::provider::ModelRef;

fn resolve_provider_model(
    provider: &ProviderModel,
    model_id: &str,
    catalog: &crate::inference::metadata::ModelCatalogSnapshot,
) -> ProviderModel {
    match provider {
        ProviderModel::OpenAI { api, params } => ProviderModel::OpenAI {
            api: Some(
                (*api)
                    .or_else(|| catalog.protocol_default("openai", model_id))
                    .unwrap_or_default(),
            ),
            params: params.clone(),
        },
        other => other.clone(),
    }
}

#[derive(Debug, Clone)]
pub struct ModelGroup {
    pub name: String,
    pub main: ModelRef,
    pub fallbacks: Vec<ModelRef>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    /// Resolved at load time by `parse_model_groups`: config override wins;
    /// else catalog's `max_input_tokens` for `main`; else `DEFAULT_CONTEXT_WINDOW`.
    pub context_window: usize,
    pub retry: RetryConfig,
    pub inference: InferenceConfig,
}

#[derive(Debug)]
pub struct ModelRegistryConfig {
    pub providers: HashMap<String, ModelProviderConfig>,
    pub models: HashMap<String, ModelGroupConfig>,
    pub skip_auto_discover: bool,
}

impl ModelRegistryConfig {
    pub fn empty() -> Self {
        Self {
            providers: HashMap::new(),
            models: HashMap::new(),
            skip_auto_discover: true,
        }
    }

    pub fn auto_discover() -> Self {
        let mut providers = HashMap::new();

        let known = [
            ("openai", "OPENAI_API_KEY"),
            ("anthropic", "ANTHROPIC_API_KEY"),
            ("groq", "GROQ_API_KEY"),
            ("openrouter", "OPENROUTER_API_KEY"),
            ("deepseek", "DEEPSEEK_API_KEY"),
            ("gemini", "GEMINI_API_KEY"),
            ("cohere", "COHERE_API_KEY"),
            ("mistral", "MISTRAL_API_KEY"),
            ("perplexity", "PERPLEXITY_API_KEY"),
            ("together", "TOGETHER_API_KEY"),
            ("xai", "XAI_API_KEY"),
            ("hyperbolic", "HYPERBOLIC_API_KEY"),
            ("moonshot", "MOONSHOT_API_KEY"),
            ("mira", "MIRA_API_KEY"),
            ("galadriel", "GALADRIEL_API_KEY"),
            ("huggingface", "HUGGINGFACE_API_KEY"),
        ];

        for (name, env_var) in known {
            if let Ok(key) = std::env::var(env_var) {
                providers.insert(
                    name.to_string(),
                    ModelProviderConfig {
                        api_key: Some(key),
                        base_url: None,
                        enabled: true,
                    },
                );
            }
        }

        if std::env::var("OLLAMA_API_BASE_URL").is_ok() {
            providers.insert(
                "ollama".to_string(),
                ModelProviderConfig {
                    api_key: None,
                    base_url: std::env::var("OLLAMA_API_BASE_URL").ok(),
                    enabled: true,
                },
            );
        }

        let inference = InferenceConfig::default();
        let models = build_default_model_groups(&providers, &inference);

        Self {
            providers,
            models,
            skip_auto_discover: false,
        }
    }

    pub fn merge_with_auto_discovered(&mut self) {
        if self.skip_auto_discover {
            return;
        }
        let discovered = Self::auto_discover();
        for (name, provider) in discovered.providers {
            self.providers.entry(name).or_insert(provider);
        }
    }

    pub fn parse_model_groups(
        &self,
        inference: &InferenceConfig,
        catalog: &crate::inference::metadata::ModelCatalogSnapshot,
    ) -> Result<HashMap<String, ModelGroup>, InferenceError> {
        let mut groups = HashMap::new();

        for (name, config) in &self.models {
            let common = config.common();
            let main = ModelRef {
                model_id: common.model.clone(),
                provider: resolve_provider_model(&config.provider, &common.model, catalog),
            };
            let fallbacks: Vec<ModelRef> = common
                .fallbacks
                .iter()
                .map(|fb| ModelRef {
                    model_id: fb.common().model.clone(),
                    provider: resolve_provider_model(&fb.provider, &fb.common().model, catalog),
                })
                .collect();

            // Only the main model's window is resolved; a tighter fallback
            // window surfaces as an inference error on its turn and falls over.
            let context_window = common.context_window.unwrap_or_else(|| {
                catalog
                    .lookup(&main)
                    .and_then(|e| e.max_input_tokens().map(|n| n as usize))
                    .unwrap_or(crate::inference::context::DEFAULT_CONTEXT_WINDOW)
            });

            groups.insert(
                name.clone(),
                ModelGroup {
                    name: name.clone(),
                    main,
                    fallbacks,
                    max_tokens: common.max_tokens,
                    temperature: common.temperature,
                    context_window,
                    retry: common.retry.clone(),
                    inference: inference.clone(),
                },
            );
        }

        Ok(groups)
    }
}

fn default_model_for_provider(provider: &str) -> &str {
    match provider {
        "anthropic" => "claude-haiku-4-5",
        "openai" => "gpt-4o",
        "groq" => "llama-3.3-70b-versatile",
        "deepseek" => "deepseek-chat",
        "gemini" => "gemini-2.0-flash",
        "mistral" => "mistral-large-latest",
        "cohere" => "command-r-plus",
        "xai" => "grok-2-latest",
        "ollama" => "qwen3-vl:32b",
        _ => "default",
    }
}

fn build_default_model_config(provider: &str, model: &str, max_tokens: u64) -> ModelGroupConfig {
    let common = CommonModelFields {
        model: model.to_string(),
        max_tokens: Some(max_tokens),
        ..Default::default()
    };
    ModelGroupConfig {
        common,
        provider: ProviderModel::from_name(provider),
    }
}

fn build_default_model_groups(
    providers: &HashMap<String, ModelProviderConfig>,
    inference: &InferenceConfig,
) -> HashMap<String, ModelGroupConfig> {
    let mut models = HashMap::new();

    if let Some((provider, _)) = providers.iter().next() {
        let model = default_model_for_provider(provider);
        models.insert(
            "primary".to_string(),
            build_default_model_config(provider, model, inference.default_max_tokens),
        );
    }

    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_ref_parse() {
        let r = ModelRef::parse("anthropic/claude-sonnet-4-5").unwrap();
        assert_eq!(r.provider_name(), "anthropic");
        assert_eq!(r.model_id, "claude-sonnet-4-5");
    }

    #[test]
    fn test_model_ref_parse_invalid() {
        assert!(ModelRef::parse("no-slash").is_err());
        assert!(ModelRef::parse("/missing-provider").is_err());
        assert!(ModelRef::parse("missing-model/").is_err());
    }

    #[test]
    fn test_model_group_config_roundtrip_anthropic() {
        let yaml = r#"
provider: anthropic
model: claude-sonnet-4-6
max_tokens: 64000
thinking:
  type: enabled
  budget_tokens: 16000
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.provider_name(), "anthropic");
        assert_eq!(config.common().model, "claude-sonnet-4-6");
        assert_eq!(config.common().max_tokens, Some(64000));
        let ProviderModel::Anthropic { params } = &config.provider else {
            panic!("expected anthropic provider");
        };
        assert_eq!(
            params
                .thinking
                .as_ref()
                .map(|thinking| thinking.budget_tokens),
            Some(Some(16000))
        );
        let written = serde_yaml::to_string(&config).unwrap();
        assert!(written.contains("provider: anthropic"));
        assert!(written.contains("model: claude-sonnet-4-6"));
        assert!(!written.contains("common:"));
        assert!(!written.contains("params:"));
    }

    #[test]
    fn test_model_group_config_roundtrip_ollama() {
        let yaml = r#"
provider: ollama
model: qwen3:32b
think: true
num_ctx: 8192
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.provider_name(), "ollama");
        assert_eq!(config.common().model, "qwen3:32b");
        let ProviderModel::Ollama { params } = &config.provider else {
            panic!("expected ollama provider");
        };
        assert_eq!(params.think, Some(true));
        assert_eq!(params.num_ctx, Some(8192));
    }

    #[test]
    fn test_model_group_config_roundtrip_openai() {
        let yaml = r#"
provider: openai
model: gpt-4o
api: responses
reasoning_effort: high
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.provider_name(), "openai");
        let ProviderModel::OpenAI { api, params } = &config.provider else {
            panic!("expected openai provider");
        };
        assert_eq!(*api, Some(crate::core::config::OpenAiApi::Responses));
        assert_eq!(params.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn test_model_group_config_roundtrip_gemini() {
        let yaml = r#"
provider: gemini
model: gemini-test
thinking_config:
  thinking_budget: 2048
  include_thoughts: true
candidate_count: 2
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        let ProviderModel::Gemini { params } = &config.provider else {
            panic!("expected gemini provider");
        };
        assert_eq!(
            params
                .thinking_config
                .as_ref()
                .map(|thinking| thinking.thinking_budget),
            Some(2048)
        );
        assert_eq!(params.candidate_count, Some(2));
        let written = serde_yaml::to_string(&config).unwrap();
        assert!(written.contains("provider: gemini"));
        assert!(!written.contains("params:"));
    }

    #[test]
    fn test_openai_compatible_provider_variants_remain_flat() {
        for name in [
            "groq",
            "openrouter",
            "deepseek",
            "xai",
            "together",
            "hyperbolic",
        ] {
            let yaml = format!("provider: {name}\nmodel: test\ntop_p: 0.75\n");
            let config: ModelGroupConfig = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(config.provider_name(), name);
            let written = serde_yaml::to_string(&config).unwrap();
            assert!(written.contains("top_p: 0.75"));
            assert!(!written.contains("params:"));
        }
    }

    #[test]
    fn test_model_group_config_with_fallbacks() {
        let yaml = r#"
provider: anthropic
model: claude-sonnet-4-6
fallbacks:
  - provider: ollama
    model: qwen3:32b
    think: true
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        let fallbacks = &config.common().fallbacks;
        assert_eq!(fallbacks.len(), 1);
        assert_eq!(fallbacks[0].provider_name(), "ollama");
        assert_eq!(fallbacks[0].common().model, "qwen3:32b");
    }

    #[test]
    fn test_model_group_config_generic_provider() {
        let yaml = r#"
provider: generic
model: some-model
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(config.provider, ProviderModel::Generic));
    }

    #[test]
    fn test_openai_protocol_resolution_precedence() {
        let yaml = r#"
provider: openai
model: configured-model
api: chat_completions
fallbacks:
  - provider: openai
    model: metadata-model
"#;
        let mut registry = ModelRegistryConfig::empty();
        registry
            .models
            .insert("primary".to_string(), serde_yaml::from_str(yaml).unwrap());
        let mut catalog = crate::inference::metadata::ModelCatalogSnapshot::empty();
        catalog.protocol_defaults.insert(
            "openai/configured-model".to_string(),
            crate::core::config::OpenAiApi::Responses,
        );
        catalog.protocol_defaults.insert(
            "openai/metadata-model".to_string(),
            crate::core::config::OpenAiApi::Responses,
        );

        let groups = registry
            .parse_model_groups(&InferenceConfig::default(), &catalog)
            .unwrap();
        let group = groups.get("primary").unwrap();
        let ProviderModel::OpenAI { api: main_api, .. } = &group.main.provider else {
            panic!("expected openai main");
        };
        let ProviderModel::OpenAI {
            api: fallback_api, ..
        } = &group.fallbacks[0].provider
        else {
            panic!("expected openai fallback");
        };
        assert_eq!(
            *main_api,
            Some(crate::core::config::OpenAiApi::ChatCompletions)
        );
        assert_eq!(
            *fallback_api,
            Some(crate::core::config::OpenAiApi::Responses)
        );
    }

    #[test]
    fn test_openai_protocol_without_metadata_uses_chat_completions() {
        let mut registry = ModelRegistryConfig::empty();
        registry.models.insert(
            "primary".to_string(),
            serde_yaml::from_str("provider: openai\nmodel: unknown-model\n").unwrap(),
        );
        let groups = registry
            .parse_model_groups(
                &InferenceConfig::default(),
                &crate::inference::metadata::ModelCatalogSnapshot::empty(),
            )
            .unwrap();
        let ProviderModel::OpenAI { api, .. } = &groups["primary"].main.provider else {
            panic!("expected openai provider");
        };
        assert_eq!(*api, Some(crate::core::config::OpenAiApi::ChatCompletions));
    }
}
