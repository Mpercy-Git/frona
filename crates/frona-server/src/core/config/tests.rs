use super::*;
use std::path::PathBuf;

#[test]
fn test_expand_env_vars() {
    unsafe { std::env::set_var("TEST_KEY_123", "my-secret") };
    let result = expand_env_vars("key=${TEST_KEY_123}");
    assert_eq!(result, "key=my-secret");
    unsafe { std::env::remove_var("TEST_KEY_123") };
}

#[test]
fn test_expand_env_vars_missing() {
    let result = expand_env_vars("key=${NONEXISTENT_VAR_XYZ}");
    assert_eq!(result, "key=");
}

#[test]
fn defaults_are_sensible() {
    let config = Config::default();
    assert_eq!(config.server.port, 3001);
    assert_eq!(
        config.auth.encryption_secret,
        "dev-secret-change-in-production"
    );
    assert_eq!(config.database.path, "data/db");
    assert_eq!(config.storage.data_dir, "data");
    assert_eq!(config.storage.skills_dir, "data/skills");
    assert_eq!(config.memory.basic_space_compaction_secs, 3600);
    assert_eq!(config.memory.pkm_consolidation_max_tool_turns, 8);
    assert_eq!(config.memory.pkm_consolidation_max_submissions, 8);
    assert_eq!(config.memory.pkm_playbook_max_tool_turns, 20);
    assert_eq!(config.memory.pkm_playbook_max_submissions, 20);
    assert!(!config.sso.enabled);
    assert!(config.sso.signups_match_email);
    assert!(config.browser.is_none());
    assert!(config.server.cors_origins.is_none());
    assert!(config.server.base_url.is_none());
    assert_eq!(config.server.max_body_size_bytes, 104_857_600);
    assert!(config.search.provider.is_none());
    assert!(config.search.searxng_base_url.is_none());
    assert_eq!(config.inference.max_tool_turns, 200);
    assert_eq!(config.inference.default_max_tokens, 8192);
    assert_eq!(config.inference.compaction_trigger_pct, 80);
    assert_eq!(config.inference.history_truncation_pct, 90);
}

#[test]
fn provider_option_serialization_omits_none_values() {
    let cases = [
        (
            "OpenAICompatParams",
            serde_json::to_value(OpenAICompatParams {
                top_p: Some(0.8),
                ..Default::default()
            })
            .unwrap(),
            serde_json::json!({ "top_p": 0.8 }),
        ),
        (
            "GeminiThinkingConfig",
            serde_json::to_value(GeminiThinkingConfig {
                thinking_budget: 1024,
                include_thoughts: None,
            })
            .unwrap(),
            serde_json::json!({ "thinking_budget": 1024 }),
        ),
        (
            "AnthropicParams",
            serde_json::to_value(AnthropicParams {
                top_k: Some(40),
                ..Default::default()
            })
            .unwrap(),
            serde_json::json!({ "top_k": 40 }),
        ),
        (
            "OllamaParams",
            serde_json::to_value(OllamaParams {
                num_ctx: Some(8192),
                ..Default::default()
            })
            .unwrap(),
            serde_json::json!({ "num_ctx": 8192 }),
        ),
        (
            "GeminiParams",
            serde_json::to_value(GeminiParams {
                candidate_count: Some(1),
                ..Default::default()
            })
            .unwrap(),
            serde_json::json!({ "candidate_count": 1 }),
        ),
        (
            "ProviderModel::OpenAI",
            serde_json::to_value(ProviderModel::OpenAI {
                api: None,
                params: OpenAICompatParams::default(),
            })
            .unwrap(),
            serde_json::json!({ "provider": "openai" }),
        ),
    ];

    for (name, actual, expected) in cases {
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn env_var_overrides_multi_word_field() {
    // The key remapping (replace first _ with __) means FRONA_BROWSER_WS_URL
    // becomes browser__ws_url, which separator("__") resolves to browser.ws_url.
    let loaded = load_with_env_override("FRONA_BROWSER_WS_URL", "ws://custom:9999");
    assert_eq!(
        loaded.config.browser.as_ref().unwrap().ws_url,
        "ws://custom:9999"
    );
}

fn load_with_env_override(key: &str, value: &str) -> LoadedConfig {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.yaml");
    std::fs::write(&path, "server:\n  port: 4321\n").unwrap();
    ConfigService::load_with_env(&path, [(key.into(), value.into())].into()).unwrap()
}

#[test]
fn env_var_overrides_server_port() {
    let loaded = load_with_env_override("FRONA_SERVER_PORT", "9999");
    assert_eq!(loaded.config.server.port, 9999);
}

#[test]
fn env_var_overrides_database_path() {
    let loaded = load_with_env_override("FRONA_DATABASE_PATH", "/tmp/testdb");
    assert_eq!(loaded.config.database.path, "/tmp/testdb");
}

#[test]
fn env_var_overrides_sso_enabled() {
    let loaded = load_with_env_override("FRONA_SSO_ENABLED", "true");
    assert!(loaded.config.sso.enabled);
}

#[test]
fn env_var_overrides_auth_allow_registration() {
    let loaded = load_with_env_override("FRONA_AUTH_ALLOW_REGISTRATION", "false");
    assert!(!loaded.config.auth.allow_registration);
}

#[test]
fn server_timezone_explicit_valid_passes() {
    let mut server = ServerConfig {
        timezone: "Asia/Tokyo".to_string(),
        ..Default::default()
    };
    resolve_server_timezone(&mut server);
    assert_eq!(server.timezone, "Asia/Tokyo");
}

#[test]
#[should_panic(expected = "Invalid server.timezone")]
fn server_timezone_explicit_invalid_panics() {
    let mut server = ServerConfig {
        timezone: "Mars/Olympus".to_string(),
        ..Default::default()
    };
    resolve_server_timezone(&mut server);
}

#[test]
fn server_timezone_empty_falls_back_to_detection() {
    let mut server = ServerConfig::default();
    assert!(server.timezone.is_empty());
    resolve_server_timezone(&mut server);
    assert!(!server.timezone.is_empty());
    assert!(
        server.timezone.parse::<chrono_tz::Tz>().is_ok(),
        "detected timezone '{}' must be a valid IANA name",
        server.timezone
    );
}

#[test]
fn auth_allow_registration_defaults_to_true() {
    let config = AuthConfig::default();
    assert!(config.allow_registration);
}

#[test]
fn browser_config_http_base_url() {
    let config = BrowserConfig {
        ws_url: "ws://localhost:3333".into(),
        ..Default::default()
    };
    assert_eq!(config.http_base_url(), "http://localhost:3333");
}

#[test]
fn browser_config_profile_path() {
    let config = BrowserConfig {
        profiles_path: "/data/profiles".into(),
        ..Default::default()
    };
    let path = config.profile_path(&crate::handle!("bob"), "github");
    assert_eq!(path, PathBuf::from("/data/profiles/bob/github"));
}

#[test]
fn mcp_cache_path_defaults_to_none() {
    let mcp = McpConfig::default();
    assert!(mcp.cache_path.is_none());
}

#[test]
fn strip_defaults_removes_all_defaults() {
    let mut value = serde_json::to_value(Config::default()).unwrap();
    strip_defaults(&mut value);
    assert_eq!(value, serde_json::json!({}));
}

#[test]
fn strip_defaults_keeps_changed_values() {
    let mut value = serde_json::json!({
        "server": { "port": 8080, "static_dir": "/app/static" },
        "auth": { "encryption_secret": "dev-secret-change-in-production" },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "server": { "port": 8080 },
        })
    );
}

#[test]
fn strip_defaults_keeps_non_default_fields() {
    let mut value = serde_json::json!({
        "server": { "cors_origins": "https://example.com" },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "server": { "cors_origins": "https://example.com" },
        })
    );
}

#[test]
fn strip_defaults_handles_integer_vs_float() {
    let mut value = serde_json::json!({
        "sandbox": { "max_cpu_pct": 95, "max_memory_pct": 80 },
    });
    strip_defaults(&mut value);
    assert_eq!(value, serde_json::json!({}));
}

#[test]
fn strip_defaults_removes_provider_entry_defaults() {
    let mut value = serde_json::json!({
        "providers": {
            "anthropic": { "base_url": null, "enabled": true },
            "openai": { "api_key": "sk-123", "enabled": true },
        },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "providers": {
                "anthropic": {},
                "openai": { "api_key": "sk-123" },
            },
        })
    );
}

#[test]
fn strip_defaults_keeps_provider_connections_when_all_fields_are_default() {
    let mut value = serde_json::json!({
        "providers": {
            "anthropic": { "base_url": null, "enabled": true },
        },
    });
    strip_defaults(&mut value);
    assert_eq!(value, serde_json::json!({"providers": {"anthropic": {}}}));
}

#[test]
fn strip_defaults_removes_model_group_entry_defaults() {
    let mut value = serde_json::json!({
        "models": {
            "coding": {
                "main": "anthropic/claude-opus-4-6",
                "fallbacks": [],
                "max_tokens": 32000,
                "temperature": null,
                "context_window": 200000,
                "retry": {
                    "max_retries": 10,
                    "initial_backoff_ms": 1000,
                    "backoff_multiplier": 2,
                    "max_backoff_ms": 60000,
                },
            },
        },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "models": {
                "coding": {
                    "main": "anthropic/claude-opus-4-6",
                    "max_tokens": 32000,
                    "context_window": 200000,
                },
            },
        })
    );
}
#[test]
fn a_provider_config_without_billing_deserializes() {
    let cfg: ModelProviderConfig =
        serde_yaml::from_str("api_key: sk-123\nenabled: true").expect("parses");
    assert!(cfg.billing.is_none());
    assert_eq!(
        cfg.effective_billing("openai").kind,
        ProviderBillingKind::Metered
    );
}

#[test]
fn a_provider_config_with_billing_deserializes() {
    let cfg: ModelProviderConfig = serde_yaml::from_str(
            "api_key: sk-123\nbilling:\n  kind: subscription\n  monthly_cost: 20\n  currency: GBP\n  included_spend_usd: 20\n  overage_is_metered: true\n",
        )
        .expect("parses");
    let billing = cfg.effective_billing("anthropic");
    assert_eq!(billing.kind, ProviderBillingKind::Subscription);
    assert_eq!(billing.monthly_cost, Some(20.0));
    assert_eq!(billing.currency_or_usd(), "GBP");
    assert!(billing.overage_is_metered);
}

/// A config that never mentions the new managed-credential/adapter fields
/// must still parse and default them to absent, so existing installs are
/// unaffected by the fields' addition.
#[test]
fn a_provider_config_without_the_new_fields_deserializes() {
    let cfg: ModelProviderConfig =
        serde_yaml::from_str("api_key: sk-123\nenabled: true").expect("parses");
    assert!(cfg.credential_id.is_none());
    assert!(cfg.provider.is_none());
    assert!(cfg.adapter.is_none());
    assert!(cfg.aws_profile.is_none());
    assert!(cfg.aws_region.is_none());
    assert!(cfg.azure_credential.is_none());
    assert!(cfg.attributes.is_empty());
}

#[test]
fn a_provider_config_with_the_new_fields_deserializes() {
    let cfg: ModelProviderConfig = serde_yaml::from_str(
            "credential_id: 3fa85f64-5717-4562-b3fc-2c963f66afa6\nprovider: azure\nadapter: openai\naws_profile: default\naws_region: us-east-1\nazure_credential: entra\nazure_api_version: 2024-10-21\n",
        )
        .expect("parses");
    assert_eq!(
        cfg.credential_id,
        Some(uuid::Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap())
    );
    assert_eq!(cfg.provider.as_deref(), Some("azure"));
    assert_eq!(cfg.adapter, Some(AdapterId::Openai));
    assert_eq!(cfg.aws_profile.as_deref(), Some("default"));
    assert_eq!(cfg.aws_region.as_deref(), Some("us-east-1"));
    assert_eq!(cfg.azure_credential.as_deref(), Some("entra"));
    assert_eq!(
        cfg.attributes
            .get("azure_api_version")
            .and_then(|v| v.as_str()),
        Some("2024-10-21")
    );
}

/// The flattened `attributes` bag must round-trip through `strip_defaults`
/// untouched — it holds operator-set data with no struct-level default to
/// compare against, so it should never be silently dropped.
#[test]
fn strip_defaults_preserves_flattened_attributes() {
    let mut value = serde_json::json!({
        "providers": {
            "azure": {
                "api_key": "sk-123",
                "enabled": true,
                "azure_api_version": "2024-10-21",
            },
        },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "providers": {
                "azure": {
                    "api_key": "sk-123",
                    "azure_api_version": "2024-10-21",
                },
            },
        })
    );
}

/// A provider whose billing an operator has stated must survive
/// `strip_defaults` — that is the round-trip `PUT /api/config` performs on
/// every save, and silently dropping the block would reclassify a
/// subscription as pay-as-you-go the next time settings were touched.
#[test]
fn strip_defaults_preserves_a_declared_billing_block() {
    let mut value = serde_json::json!({
        "providers": {
            "anthropic": {
                "api_key": "sk-123",
                "enabled": true,
                "billing": { "kind": "subscription", "monthly_cost": 20.0, "overage_is_metered": false },
            },
        },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "providers": {
                "anthropic": {
                    "api_key": "sk-123",
                    "billing": { "kind": "subscription", "monthly_cost": 20.0, "overage_is_metered": false },
                },
            },
        })
    );
}

/// The other half: a config that never mentioned billing must come back
/// out exactly as it went in, so adding the field changes nothing for
/// existing installs.
#[test]
fn strip_defaults_leaves_a_config_without_billing_untouched() {
    let mut value = serde_json::json!({
        "providers": { "openai": { "api_key": "sk-123", "enabled": true } },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({ "providers": { "openai": { "api_key": "sk-123" } } })
    );
}

/// `provider` is the serde tag of the flattened `ProviderModel`, so a model
/// group that loses it no longer deserializes. It used to be stripped
/// whenever it equalled the default variant (`generic`), which meant saving
/// an OpenAI-compatible model group from the settings UI wrote a config.yaml
/// that panicked the next startup with
/// `missing configuration field "models.primary.provider"`.
#[test]
fn strip_defaults_keeps_provider_tag_matching_the_default_variant() {
    let mut value = serde_json::json!({
        "models": {
            "primary": {
                "provider": "generic",
                "model": "qwen3-coder",
                "fallbacks": [],
                "temperature": null,
            },
        },
    });
    strip_defaults(&mut value);
    assert_eq!(
        value,
        serde_json::json!({
            "models": {
                "primary": { "provider": "generic", "model": "qwen3-coder" },
            },
        })
    );
}

/// Every credential in the config must be redacted on the way out. The SMTP
/// password is the one most recently added, and `GET /api/config` is
/// reachable by any authenticated user — not just admins.
#[test]
fn smtp_password_is_redacted_for_api_and_logs() {
    let mut config = Config::default();
    config.mail.smtp_password = Some("hunter2-smtp".into());

    let mut api_value = serde_json::to_value(&config).unwrap();
    redact_config_for_api(&mut api_value);
    let rendered = serde_json::to_string(&api_value).unwrap();
    assert!(
        !rendered.contains("hunter2-smtp"),
        "API response leaked the SMTP password"
    );
    assert_eq!(
        api_value.pointer("/mail/smtp_password/is_set"),
        Some(&serde_json::Value::Bool(true))
    );

    let mut log_value = serde_json::to_value(&config).unwrap();
    redact_config_for_log(&mut log_value);
    let rendered = serde_json::to_string(&log_value).unwrap();
    assert!(
        !rendered.contains("hunter2-smtp"),
        "log dump leaked the SMTP password"
    );
}

#[test]
fn unset_smtp_password_reports_as_not_set() {
    let config = Config::default();
    let mut api_value = serde_json::to_value(&config).unwrap();
    redact_config_for_api(&mut api_value);
    // Absent secrets must not masquerade as configured ones.
    assert_ne!(
        api_value.pointer("/mail/smtp_password/is_set"),
        Some(&serde_json::Value::Bool(true))
    );
}

#[test]
fn apps_url_unset_keeps_apps_on_the_main_origin() {
    let server = ServerConfig::default();
    assert!(server.public_apps_url().is_none());
    assert!(server.apps_host().is_none());
    assert!(server.validate_apps_url().is_ok());
}

#[test]
fn apps_url_is_normalised_to_an_origin() {
    let server = ServerConfig {
        base_url: Some("https://frona.example.com".into()),
        apps_url: Some("https://Apps.Example.com:8443/ignored/path/".into()),
        ..Default::default()
    };
    assert_eq!(
        server.public_apps_url().as_deref(),
        Some("https://apps.example.com:8443")
    );
    assert_eq!(server.apps_host().as_deref(), Some("apps.example.com:8443"));
    assert!(server.validate_apps_url().is_ok());
}

#[test]
fn apps_url_must_be_a_different_origin_and_need_a_base_url() {
    let same = ServerConfig {
        base_url: Some("https://frona.example.com".into()),
        apps_url: Some("https://frona.example.com".into()),
        ..Default::default()
    };
    assert!(same.validate_apps_url().is_err());

    let no_base = ServerConfig {
        apps_url: Some("https://apps.example.com".into()),
        ..Default::default()
    };
    assert!(no_base.validate_apps_url().is_err());

    let garbage = ServerConfig {
        base_url: Some("https://frona.example.com".into()),
        apps_url: Some("not a url".into()),
        ..Default::default()
    };
    assert!(garbage.validate_apps_url().is_err());

    let blank = ServerConfig {
        apps_url: Some("  ".into()),
        ..Default::default()
    };
    assert!(blank.validate_apps_url().is_ok());
}
