//! Context windows reported by a provider's own model listing.
//!
//! The static catalog lags new releases: a model it has never seen resolves to
//! the 128K fallback, which then rejects requests the model could serve. When
//! the provider's live listing reports a context length for a model, that is
//! the authoritative number; the catalog value stays as the fallback for
//! providers that cannot list, do not report one, or are unreachable.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;

use crate::inference::provider::{ModelProvider, ProviderModelList, group::ModelGroup};

/// How long startup waits for any one provider's listing before keeping the
/// catalog value. Listings run concurrently, so this bounds the total delay.
pub const LISTING_TIMEOUT: Duration = Duration::from_secs(5);

/// Model id -> context length, or `None` when the provider cannot list, fails,
/// or is too slow. Failures are not errors here: the catalog value stands.
async fn listed_windows(
    handle: &str,
    provider: Arc<dyn ModelProvider>,
    timeout: Duration,
) -> Option<HashMap<String, usize>> {
    match tokio::time::timeout(timeout, provider.list_models()).await {
        Ok(Ok(ProviderModelList::Listed { models, .. })) => Some(
            models
                .data
                .into_iter()
                .filter_map(|model| {
                    model
                        .context_length
                        .filter(|length| *length > 0)
                        .map(|length| (model.id, length as usize))
                })
                .collect(),
        ),
        Ok(Ok(ProviderModelList::CatalogFallback)) => None,
        Ok(Err(error)) => {
            tracing::debug!(provider = handle, %error, "Model listing failed; keeping catalog context windows");
            None
        }
        Err(_) => {
            tracing::debug!(
                provider = handle,
                "Model listing timed out; keeping catalog context windows"
            );
            None
        }
    }
}

/// Replace each group's context window with the one its provider's listing
/// reports for the group's main model. Groups named in `explicit` carry a
/// configured `context_window`, which always wins and is never touched.
pub async fn apply_listed_context_windows(
    groups: &mut HashMap<String, ModelGroup>,
    explicit: &HashSet<String>,
    providers: &HashMap<String, Arc<dyn ModelProvider>>,
    timeout: Duration,
) {
    let wanted: BTreeSet<&str> = groups
        .iter()
        .filter(|(name, _)| !explicit.contains(*name))
        .map(|(_, group)| group.main.provider_handle.as_str())
        .collect();
    let listings: HashMap<String, HashMap<String, usize>> =
        join_all(wanted.into_iter().filter_map(|handle| {
            let provider = providers.get(handle)?.clone();
            Some(async move {
                let listed = listed_windows(handle, provider, timeout).await;
                (handle.to_string(), listed)
            })
        }))
        .await
        .into_iter()
        .filter_map(|(handle, listed)| listed.map(|listed| (handle, listed)))
        .collect();

    for (name, group) in groups.iter_mut() {
        if explicit.contains(name) {
            continue;
        }
        let Some(window) = listings
            .get(group.main.provider_handle.as_str())
            .and_then(|listed| listed.get(&group.main.model_id))
        else {
            continue;
        };
        if *window != group.context_window {
            tracing::info!(
                group = %name,
                model = %group.main.model_id,
                catalog = group.context_window,
                listed = *window,
                "Using the provider-reported context window"
            );
            group.context_window = *window;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chat::broadcast::BroadcastService,
        core::{
            Handle,
            config::{ModelProviderConfig, ProviderModel},
        },
        inference::provider::{
            InferenceCounter, ModelConfig, ModelRequestSettings,
            platform::{ProviderPlatform, build_provider},
        },
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn provider(brand: &str, endpoint: &str) -> Arc<dyn ModelProvider> {
        let config = ModelProviderConfig {
            provider: Some(brand.into()),
            base_url: Some(endpoint.into()),
            api_key: Some("fixture-key".into()),
            ..Default::default()
        };
        let connection =
            ProviderPlatform::resolve(&Handle::const_validated("openrouter"), &config).unwrap();
        build_provider(
            &connection,
            &config,
            &InferenceCounter::new(BroadcastService::new()),
        )
        .unwrap()
    }

    fn group(name: &str, provider: &str, model: &str, window: usize) -> ModelGroup {
        ModelGroup {
            name: name.into(),
            main: ModelConfig {
                catalog_provider: provider.into(),
                provider_handle: Handle::try_new(provider).unwrap(),
                model_id: model.into(),
                provider: ProviderModel::default(),
                request_settings: ModelRequestSettings::default(),
            },
            fallbacks: vec![],
            max_tokens: None,
            temperature: None,
            context_window: window,
            retry: Default::default(),
            inference: Default::default(),
            providers: Default::default(),
        }
    }

    async fn openrouter(
        body: serde_json::Value,
        status: u16,
    ) -> (MockServer, Arc<dyn ModelProvider>) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        let provider = provider("openrouter", &server.uri());
        (server, provider)
    }

    fn providers(provider: Arc<dyn ModelProvider>) -> HashMap<String, Arc<dyn ModelProvider>> {
        HashMap::from([("openrouter".to_string(), provider)])
    }

    fn fixture_groups(window: usize) -> HashMap<String, ModelGroup> {
        HashMap::from([
            (
                "new".into(),
                group("new", "openrouter", "vendor/new-model", window),
            ),
            (
                "old".into(),
                group("old", "openrouter", "vendor/old-model", window),
            ),
            (
                "pinned".into(),
                group("pinned", "openrouter", "vendor/new-model", window),
            ),
        ])
    }

    #[tokio::test]
    async fn the_providers_listing_replaces_the_catalog_fallback() {
        let (_server, provider) = openrouter(
            serde_json::json!({"data": [
                {"id": "vendor/new-model", "name": "New", "created": 1, "context_length": 1_000_000},
                {"id": "vendor/old-model", "name": "Old", "created": 1},
            ]}),
            200,
        )
        .await;
        let mut groups = fixture_groups(128_000);
        let explicit = HashSet::from(["pinned".to_string()]);
        apply_listed_context_windows(
            &mut groups,
            &explicit,
            &providers(provider),
            Duration::from_secs(5),
        )
        .await;
        // Reported by the provider: used.
        assert_eq!(groups["new"].context_window, 1_000_000);
        // Listed without a context length: the catalog value stands.
        assert_eq!(groups["old"].context_window, 128_000);
        // An explicit configured window always wins over the listing.
        assert_eq!(groups["pinned"].context_window, 128_000);
    }

    #[tokio::test]
    async fn an_unavailable_listing_keeps_the_catalog_value() {
        let (_server, provider) = openrouter(serde_json::json!({"error": "down"}), 500).await;
        let mut groups = fixture_groups(128_000);
        apply_listed_context_windows(
            &mut groups,
            &HashSet::new(),
            &providers(provider),
            Duration::from_secs(5),
        )
        .await;
        assert!(groups.values().all(|group| group.context_window == 128_000));
    }

    #[tokio::test]
    async fn a_missing_provider_keeps_the_catalog_value() {
        let mut groups = fixture_groups(128_000);
        apply_listed_context_windows(
            &mut groups,
            &HashSet::new(),
            &HashMap::new(),
            Duration::from_secs(5),
        )
        .await;
        assert!(groups.values().all(|group| group.context_window == 128_000));
    }
}
