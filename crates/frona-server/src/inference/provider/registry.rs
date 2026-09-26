use std::{collections::HashMap, sync::Arc};

use crate::core::Handle;
use crate::core::config::{InferenceConfig, ModelProviderConfig, ProviderModel, RetryConfig};
use crate::inference::{
    error::InferenceError,
    provider::{
        InferenceCounter, ModelConfig, ModelProvider, ModelRef, ModelRequestSettings,
        group::ModelGroup,
    },
};

/// Parse an ad-hoc "provider/model" reference (e.g. from a slash command or a
/// user-typed model override), as distinct from `ModelRef`, which names a
/// *configured model group* ("primary", "title", ...).
fn parse_ad_hoc(s: &str) -> Result<ModelConfig, InferenceError> {
    let (provider, model_id) = s.split_once('/').ok_or_else(|| {
        InferenceError::ConfigError(format!("expected 'provider/model' format, got '{s}'"))
    })?;
    if provider.is_empty() || model_id.is_empty() {
        return Err(InferenceError::ConfigError(format!(
            "provider and model must be non-empty, got '{s}'"
        )));
    }
    Ok(ModelConfig {
        catalog_provider: provider.to_string(),
        provider_handle: Handle::try_new(provider)
            .map_err(|error| InferenceError::ConfigError(error.to_string()))?,
        model_id: model_id.to_string(),
        provider: ProviderModel::from_name(provider),
        request_settings: ModelRequestSettings::default(),
    })
}

#[derive(Clone)]
pub struct ModelProviderRegistry {
    providers: Arc<HashMap<String, Arc<dyn ModelProvider>>>,
    model_groups: Arc<HashMap<String, ModelGroup>>,
    protocol_defaults: Arc<HashMap<String, crate::core::config::OpenAiApi>>,
    inference: InferenceConfig,
}

impl ModelProviderRegistry {
    pub fn from_config(
        config: crate::inference::config::ModelRegistryConfig,
        broadcast: crate::chat::broadcast::BroadcastService,
        inference: &InferenceConfig,
        catalog: &frona_model_catalog::ModelCatalogSnapshot,
    ) -> Result<Self, InferenceError> {
        let counter = InferenceCounter::new(broadcast);
        let mut providers: HashMap<String, Arc<dyn ModelProvider>> = HashMap::new();
        for (name, entry) in &config.providers {
            if !entry.enabled {
                tracing::info!(provider = %name, "Provider disabled, skipping");
                continue;
            }
            match super::platform::build_provider(
                &super::platform::ProviderPlatform::resolve(name, entry)?,
                entry,
                &counter,
            ) {
                Ok(provider) => {
                    tracing::info!(provider = %name, "Provider initialized");
                    providers.insert(name.as_str().to_string(), provider);
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
        let providers = Arc::new(providers);
        let model_groups =
            config.parse_model_groups_with_catalog(inference, catalog, providers.clone())?;
        Ok(Self {
            providers,
            model_groups: Arc::new(model_groups),
            // `catalog.protocol_defaults` carries raw npm labels
            // (`frona_model_catalog` is provider-execution-agnostic); resolve
            // to the wire protocol this fork's OpenAI adapter actually needs.
            protocol_defaults: Arc::new(
                catalog
                    .protocol_defaults
                    .iter()
                    .filter_map(|(key, npm)| {
                        crate::inference::metadata::catalog::openai_api_from_npm(npm)
                            .map(|api| (key.clone(), api))
                    })
                    .collect(),
            ),
            inference: inference.clone(),
        })
    }

    pub(crate) fn providers(&self) -> &Arc<HashMap<String, Arc<dyn ModelProvider>>> {
        &self.providers
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

    /// Resolve either a named model group ("primary") or an ad-hoc
    /// "provider/model" reference (e.g. from a slash command).
    pub fn resolve_model_group(&self, name_or_ref: &str) -> Result<ModelGroup, InferenceError> {
        if name_or_ref.contains('/') {
            let mut model = parse_ad_hoc(name_or_ref)?;
            let protocol_key = model.as_str();
            if let ProviderModel::OpenAI { api, .. } = &mut model.provider {
                *api = Some(
                    self.protocol_defaults
                        .get(&protocol_key)
                        .copied()
                        .unwrap_or_default(),
                );
            }
            Ok(ModelGroup {
                providers: self.providers.clone(),
                name: name_or_ref.to_string(),
                main: model,
                fallbacks: vec![],
                max_tokens: Some(self.inference.default_max_tokens),
                temperature: None,
                // Ad-hoc model reference (e.g. from a slash command). No
                // catalog lookup at this layer - fall back to the
                // conservative default. Callers that want a precise window
                // should configure a proper ModelGroup.
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

    pub fn resolve(&self, reference: &ModelRef) -> Result<ModelGroup, InferenceError> {
        self.resolve_model_group(reference.as_str())
    }

    pub fn resolve_with_fallback(
        &self,
        reference: &ModelRef,
        fallback: &ModelRef,
    ) -> Result<ModelGroup, InferenceError> {
        match self.resolve(reference) {
            Err(error) if !reference.as_str().is_empty() => {
                self.resolve(fallback).map_err(|_| error)
            }
            result => result,
        }
    }

    pub async fn unavailable_models(&self) -> HashMap<String, Vec<(String, String)>> {
        let mut unavailable = HashMap::new();
        for (name, group) in self.model_groups.iter() {
            let mut reasons = Vec::new();
            for model in std::iter::once(&group.main).chain(&group.fallbacks) {
                if let Err(error) = async {
                    let provider = self.providers.get(model.provider_name()).ok_or_else(|| {
                        InferenceError::ProviderNotConfigured(model.provider_name().into())
                    })?;
                    provider.ensure_usable(model).await
                }
                .await
                {
                    reasons.push((model.as_str(), error.to_string()));
                }
            }
            if !reasons.is_empty() {
                unavailable.insert(name.clone(), reasons);
            }
        }
        unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_groups_and_lookup_only_copies_the_selected_group() {
        let config: crate::core::config::Config = serde_json::from_value(serde_json::json!({
            "providers":{"work":{"provider":"openai"}},
            "models":{
                "primary":{"provider":"work","model":"first","extra_params":{"nested":{"values":[1,2,3]}},
                    "fallbacks":[{"provider":"work","model":"backup"}]},
                "title":{"provider":"work","model":"second"}
            }
        })).unwrap();
        let groups = crate::inference::config::ModelRegistryConfig {
            providers: config.providers,
            models: config.models,
            skip_auto_discover: true,
        }
        .parse_model_groups(&config.inference, Default::default())
        .unwrap();
        let registry = ModelProviderRegistry::for_testing(HashMap::new(), groups);
        let clone = registry.clone();
        assert!(Arc::ptr_eq(&registry.model_groups, &clone.model_groups));
        assert!(Arc::ptr_eq(&registry.providers, &clone.providers));
        let mut selected = clone.resolve(&ModelRef::PRIMARY).unwrap();
        selected.main.request_settings.extra_params.clear();
        selected.fallbacks.clear();
        let original = registry.resolve(&ModelRef::PRIMARY).unwrap();
        assert_eq!(original.fallbacks.len(), 1);
        assert!(
            original
                .main
                .request_settings
                .extra_params
                .contains_key("nested")
        );
        assert_eq!(
            registry.resolve(&ModelRef::TITLE).unwrap().main.model_id,
            "second"
        );
    }
}
