use std::collections::HashMap;

pub use crate::core::config::{
    CommonModelFields, InferenceConfig, ModelGroupConfig, ModelProviderConfig, RetryConfig,
};

use super::error::InferenceError;
use super::provider::ModelRef;

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

        Self { providers, models, skip_auto_discover: false }
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
                provider: config.provider_name().to_string(),
                model_id: common.model.clone(),
                additional_params: config.additional_params(),
            };
            let fallbacks: Vec<ModelRef> = common
                .fallbacks
                .iter()
                .map(|fb| ModelRef {
                    provider: fb.provider_name().to_string(),
                    model_id: fb.common().model.clone(),
                    additional_params: fb.additional_params(),
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
    match provider {
        "anthropic" => ModelGroupConfig::Anthropic { common, thinking: None, top_p: None, top_k: None, stop_sequences: None },
        "ollama" => ModelGroupConfig::Ollama {
            common, think: None, num_ctx: None, num_predict: None, num_batch: None,
            num_keep: None, num_thread: None, num_gpu: None, top_k: None, top_p: None,
            min_p: None, repeat_penalty: None, repeat_last_n: None,
            frequency_penalty: None, presence_penalty: None, mirostat: None,
            mirostat_eta: None, mirostat_tau: None, tfs_z: None, seed: None,
            stop: None, use_mmap: None, use_mlock: None,
        },
        "openai" => ModelGroupConfig::OpenAI { common, params: Default::default() },
        "groq" => ModelGroupConfig::Groq { common, params: Default::default() },
        "openrouter" => ModelGroupConfig::OpenRouter { common, params: Default::default() },
        "deepseek" => ModelGroupConfig::DeepSeek { common, params: Default::default() },
        "xai" => ModelGroupConfig::XAI { common, params: Default::default() },
        "together" => ModelGroupConfig::Together { common, params: Default::default() },
        "hyperbolic" => ModelGroupConfig::Hyperbolic { common, params: Default::default() },
        "gemini" => ModelGroupConfig::Gemini { common, thinking_config: None, top_p: None, top_k: None, stop_sequences: None, candidate_count: None },
        _ => ModelGroupConfig::Generic { common },
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
        assert_eq!(r.provider, "anthropic");
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
        let params = config.additional_params().unwrap();
        assert!(params.get("thinking").is_some());
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
        let params = config.additional_params().unwrap();
        assert_eq!(params.get("think").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(params.get("num_ctx").and_then(|v| v.as_u64()), Some(8192));
    }

    #[test]
    fn test_model_group_config_roundtrip_openai() {
        let yaml = r#"
provider: openai
model: gpt-4o
reasoning_effort: high
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.provider_name(), "openai");
        let params = config.additional_params().unwrap();
        assert_eq!(params.get("reasoning_effort").and_then(|v| v.as_str()), Some("high"));
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
    fn test_model_group_config_no_params_returns_none() {
        let yaml = r#"
provider: generic
model: some-model
"#;
        let config: ModelGroupConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.additional_params().is_none());
    }
}
