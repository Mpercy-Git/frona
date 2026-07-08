use frona::core::config::{deep_merge, redact_config_for_api, redact_config_for_log, config_file_path, Config};
use serde_json::json;

#[test]
fn test_config_file_path_default() {
    // Without env vars set, returns data/config.yaml
    let path = config_file_path();
    assert!(path.ends_with("config.yaml"));
}

#[test]
fn test_redact_config_for_log() {
    let config = Config::default();
    let mut value = serde_json::to_value(&config).unwrap();
    redact_config_for_log(&mut value);

    assert_eq!(
        value["auth"]["encryption_secret"],
        json!("[redacted]")
    );
}

#[test]
fn test_redact_config_for_api() {
    let config = Config::default();
    let mut value = serde_json::to_value(&config).unwrap();
    redact_config_for_api(&mut value);

    // encryption_secret is the default placeholder, so is_set should be false
    assert_eq!(
        value["auth"]["encryption_secret"],
        json!({"is_set": false})
    );

    // sso.client_secret is None by default, so is_set should be false
    assert_eq!(
        value["sso"]["client_secret"],
        json!({"is_set": false})
    );
}

#[test]
fn test_redact_config_for_api_providers() {
    let mut config = Config::default();
    config.providers.insert("anthropic".into(), frona::core::config::ModelProviderConfig {
        api_key: Some("sk-secret".into()),
        base_url: None,
        enabled: true,
    });

    let mut value = serde_json::to_value(&config).unwrap();
    redact_config_for_api(&mut value);

    assert_eq!(
        value["providers"]["anthropic"]["api_key"],
        json!({"is_set": true})
    );
    // enabled should not be redacted
    assert_eq!(value["providers"]["anthropic"]["enabled"], json!(true));
}

#[test]
fn test_deep_merge_objects() {
    let mut base = json!({"a": 1, "b": {"c": 2, "d": 3}});
    let patch = json!({"b": {"c": 42}});
    deep_merge(&mut base, patch);
    assert_eq!(base, json!({"a": 1, "b": {"c": 42, "d": 3}}));
}

#[test]
fn test_deep_merge_null_removes_key() {
    let mut base = json!({"a": 1, "b": 2});
    let patch = json!({"b": null});
    deep_merge(&mut base, patch);
    assert_eq!(base, json!({"a": 1}));
}

#[test]
fn test_deep_merge_skips_redaction_markers() {
    let mut base = json!({"auth": {"encryption_secret": "real-secret"}});
    let patch = json!({"auth": {"encryption_secret": {"is_set": true}}});
    deep_merge(&mut base, patch);
    assert_eq!(base["auth"]["encryption_secret"], json!("real-secret"));
}

#[test]
fn test_deep_merge_overwrites_non_objects() {
    let mut base = json!({"a": "old"});
    let patch = json!({"a": "new", "b": 42});
    deep_merge(&mut base, patch);
    assert_eq!(base, json!({"a": "new", "b": 42}));
}

#[test]
fn test_json_schema_generation() {
    let schema = schemars::schema_for!(Config);
    let value = serde_json::to_value(schema).unwrap();

    // Should have properties for top-level config sections
    let props = value["properties"].as_object().unwrap();
    assert!(props.contains_key("server"));
    assert!(props.contains_key("auth"));
    assert!(props.contains_key("providers"));
    assert!(props.contains_key("models"));

    // Server port should have a description
    let server_ref = &value["properties"]["server"];
    // Navigate through $ref to find server properties
    assert!(server_ref.is_object());
}

#[tokio::test]
async fn test_runtime_config_operations() {
    let db = surrealdb::Surreal::new::<surrealdb::engine::local::Mem>(())
        .await
        .unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();

    let metrics_handle = frona::core::metrics::setup_metrics_recorder();
    let config = Config::default();
    let storage = frona::storage::StorageService::new(&config);
    let resource_manager = std::sync::Arc::new(
        frona::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(80.0, 80.0, 90.0, 90.0),
    );
    let state = frona::core::state::AppState::new(db, &config, Some(frona::inference::config::ModelRegistryConfig::empty()), storage, metrics_handle, resource_manager);

    // Initially not set
    let val = state.get_runtime_config("setup_completed").await.unwrap();
    assert!(val.is_none());
    assert!(!state.get_runtime_config_bool("setup_completed").await);

    // Set the flag
    state.set_runtime_config("setup_completed", "true").await.unwrap();
    assert!(state.get_runtime_config_bool("setup_completed").await);

    // Overwrite
    state.set_runtime_config("setup_completed", "false").await.unwrap();
    assert!(!state.get_runtime_config_bool("setup_completed").await);

    // Different key
    state.set_runtime_config("other_flag", "hello").await.unwrap();
    let val = state.get_runtime_config("other_flag").await.unwrap();
    assert_eq!(val, Some("hello".to_string()));
}

/// Regression: https://github.com/fronalabs/frona/issues/27
///
/// `persist_config` strips fields equal to `Config::default()` for compactness.
/// `Config::load()` then has to be able to reconstruct them — partial structs
/// on disk must deserialize. Before fixing this, editing a single
/// `RetryConfig` field through the GUI persisted a partial `retry: {...}` and
/// crashed the server on next startup.
#[test]
fn retry_config_survives_strip_defaults_round_trip() {
    use frona::core::config::persist_config;

    let mut value = json!({
        "auth": { "encryption_secret": "aaaa" },
        "providers": { "openrouter": { "api_key": "sk-or-test" } },
        "models": {
            "primary": {
                "provider": "openrouter",
                "model": "google/gemma-4-31b-it:free",
                "retry": {
                    "max_retries": 3,
                    "initial_backoff_ms": 1000,
                    "backoff_multiplier": 2.0,
                    "max_backoff_ms": 60000
                }
            }
        }
    });

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.yaml").to_string_lossy().into_owned();

    persist_config(&mut value, &path).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();

    // strip_defaults removes the three fields that match RetryConfig::default().
    assert!(written.contains("max_retries: 3"));
    assert!(!written.contains("initial_backoff_ms"));

    let loaded: Config = ::config::Config::builder()
        .add_source(::config::File::from_str(&written, ::config::FileFormat::Yaml))
        .build()
        .unwrap()
        .try_deserialize()
        .expect("load must succeed after persist trims retry fields");

    let primary = loaded.models.get("primary").expect("primary model present");
    let retry = match primary {
        frona::core::config::ModelGroupConfig::OpenRouter { common, .. } => &common.retry,
        other => panic!("unexpected variant: {other:?}"),
    };
    assert_eq!(retry.max_retries, 3);
    assert_eq!(retry.initial_backoff_ms, 1000);
    assert_eq!(retry.backoff_multiplier, 2.0);
    assert_eq!(retry.max_backoff_ms, 60000);
}

/// Guards every persisted config struct against the same round-trip trap as
/// `retry_config_survives_strip_defaults_round_trip`. The default Config is
/// the worst case for `strip_defaults` — every field matches the default,
/// so strip removes everything except map entries it can't compare. The
/// stripped output must still load back into an equivalent Config.
#[test]
fn default_config_survives_strip_defaults_round_trip() {
    use frona::core::config::persist_config;

    // Set one non-default field per vulnerable struct so the entry survives
    // strip_defaults and we exercise the deserializer with a partial shape
    // (the other sibling fields get stripped).
    let mut value = json!({
        "search": { "provider": "tavily" },
        "signal": { "max_pending_per_user": 99 },
        "providers": {
            "openrouter": { "api_key": "sk-or-x" }
        },
        "models": {
            "primary": {
                "provider": "anthropic",
                "model": "claude-3-5-sonnet",
                "thinking": { "type": "enabled", "budget_tokens": 1000 }
            }
        }
    });

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.yaml").to_string_lossy().into_owned();
    persist_config(&mut value, &path).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();

    let loaded: Config = ::config::Config::builder()
        .add_source(::config::File::from_str(&written, ::config::FileFormat::Yaml))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap_or_else(|e| panic!("load failed: {e}\n--- written ---\n{written}"));

    assert_eq!(loaded.search.provider.as_deref(), Some("tavily"));
    assert_eq!(loaded.signal.max_pending_per_user, 99);
    assert!(loaded.providers.contains_key("openrouter"));
    assert!(loaded.models.contains_key("primary"));
}
