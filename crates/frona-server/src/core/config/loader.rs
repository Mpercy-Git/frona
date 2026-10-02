use super::*;
use crate::core::{Handle, error::AppError};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

const ENV_PREFIX: &str = "FRONA_";

const EXCLUDED_ENV_VARS: &[&str] = &[
    "FRONA_CONFIG",
    "FRONA_LOG_CONFIG",
    "FRONA_LOG_LEVEL",
    "FRONA_SERVER_DATA_DIR",
];

pub struct LoadedConfig {
    pub(super) path: PathBuf,
    pub(super) revision: String,
    pub(super) defaults: Config,
    pub config: Config,
    pub models: Option<crate::inference::config::ModelRegistryConfig>,
}

impl LoadedConfig {
    /// Revision hash of the on-disk document this snapshot was built from -
    /// the same hash `ConfigService::persisted`/`save` compute, so a caller
    /// outside `core::config` (e.g. the `GET /api/config` route) can report it
    /// as `persisted_revision` without reaching into a `pub(super)` field.
    pub fn revision(&self) -> &str {
        &self.revision
    }
}

impl ConfigService {
    /// Read a startup snapshot without a database. A missing file uses defaults
    /// and environment overrides; malformed or unreadable input is an error.
    pub fn load(path: impl AsRef<Path>) -> Result<LoadedConfig, AppError> {
        Self::load_with_env(path, std::env::vars().collect())
    }

    pub(super) fn load_with_env(
        path: impl AsRef<Path>,
        env: HashMap<String, String>,
    ) -> Result<LoadedConfig, AppError> {
        let config_path = path.as_ref().to_path_buf();
        let bytes = super::document::read_file(&config_path)?;
        let revision = super::document::revision(&bytes);
        let yaml_content = if bytes.is_empty() {
            None
        } else {
            Some(
                String::from_utf8(bytes)
                    .map_err(|error| AppError::Validation(error.to_string()))?,
            )
        };

        let defaults = Self::resolve_yaml_with_env(None, env.clone(), &config_path)?;
        let mut config = Self::resolve_yaml_with_env(yaml_content.as_deref(), env, &config_path)?;
        resolve_server_timezone(&mut config.server);

        let models = if !config.models.is_empty() || !config.providers.is_empty() {
            Some(crate::inference::config::ModelRegistryConfig {
                providers: config.providers.clone().into_iter().collect(),
                models: config.models.clone().into_iter().collect(),
                skip_auto_discover: false,
            })
        } else {
            None
        };

        if yaml_content.is_some() {
            tracing::info!(path = %config_path.display(), "Loaded config from YAML");
        } else {
            tracing::info!("No config file found, using defaults and env vars");
        }

        if let Ok(mut v) = serde_json::to_value(&config) {
            redact_config_for_log(&mut v);
            tracing::debug!(
                "Effective config:\n{}",
                serde_json::to_string_pretty(&v).unwrap_or_default()
            );
        }

        Ok(LoadedConfig {
            config,
            defaults,
            models,
            path: config_path,
            revision,
        })
    }

    fn resolve_yaml_with_env(
        yaml_content: Option<&str>,
        env: HashMap<String, String>,
        config_path: &Path,
    ) -> Result<Config, AppError> {
        let data_dir = env
            .get("FRONA_SERVER_DATA_DIR")
            .cloned()
            .unwrap_or_else(|| "data".into());

        let mut builder = config::Config::builder()
            .set_default("database.path", format!("{data_dir}/db"))
            .unwrap()
            .set_default("storage.data_dir", data_dir.clone())
            .unwrap()
            .set_default("storage.skills_dir", format!("{data_dir}/skills"))
            .unwrap()
            .set_default("storage.cache_dir", format!("{data_dir}/system/cache"))
            .unwrap()
            .set_default("storage.ontology_dir", format!("{data_dir}/ontology"))
            .unwrap();

        if let Some(content) = yaml_content {
            let expanded = expand_config_env_vars(content)?;
            builder =
                builder.add_source(config::File::from_str(&expanded, config::FileFormat::Yaml));
        }

        // FRONA_BROWSER_WS_URL -> browser__ws_url -> browser.ws_url
        let frona_env: HashMap<String, String> = env
            .into_iter()
            .filter(|(k, _)| k.starts_with(ENV_PREFIX) && !EXCLUDED_ENV_VARS.contains(&k.as_str()))
            .map(|(k, v)| {
                let stripped = k[ENV_PREFIX.len()..].to_lowercase();
                let mapped = match stripped.find('_') {
                    Some(pos) => format!("{}__{}", &stripped[..pos], &stripped[pos + 1..]),
                    None => stripped,
                };
                (mapped, v)
            })
            .collect();

        builder = builder.add_source(
            config::Environment::default()
                .source(Some(frona_env))
                .separator("__")
                .try_parsing(true),
        );

        let built = builder.build().map_err(|error| {
            AppError::Validation(config_load_error(&error.to_string(), config_path))
        })?;

        let mut config: Config = built.try_deserialize().map_err(|error| {
            AppError::Validation(config_load_error(&error.to_string(), config_path))
        })?;

        // Preserve provider credential source references for runtime precedence.
        // Other configuration fields retain the existing expansion behavior.
        if let Some(content) = yaml_content {
            let raw: serde_yaml::Value = serde_yaml::from_str(content)
                .map_err(|error| AppError::Validation(error.to_string()))?;
            if let Some(providers) = raw.get("providers").and_then(serde_yaml::Value::as_mapping) {
                for (name, provider) in providers {
                    let Some(name) = name.as_str() else { continue };
                    let Some(key) = provider.get("api_key").and_then(serde_yaml::Value::as_str)
                    else {
                        continue;
                    };
                    if key.starts_with("${") && key.ends_with('}') {
                        let handle = Handle::try_new(name)?;
                        if let Some(entry) = config.providers.get_mut(&handle) {
                            entry.api_key = Some(key.to_string());
                        }
                    }
                }
            }
        }

        Ok(config)
    }
}

/// A config error is the operator's to act on - name the file being read, and
/// for the mistakes that have an obvious fix, say what to write. Ported from
/// the pre-`ConfigService` `config.rs`'s `config_load_error`/
/// `missing_provider_group`: ad-hoc-edited or older-build config.yaml files
/// commonly omit a model group's `provider:` tag, so that specific case gets
/// a worked example rather than the raw deserialize error alone.
fn config_load_error(err: &str, path: &Path) -> String {
    let mut msg = format!("Failed to load config from {}: {err}", path.display());

    if let Some(group) = missing_provider_group(err) {
        msg.push_str(&format!(
            "\n\nhint: every model group needs a `provider:` naming its backend:\
             \n\n  models:\
             \n    {group}:\
             \n      provider: anthropic   # or openai, openrouter, gemini, groq, azure, ollama, ...\
             \n      model: <model id>\
             \n\nUse `provider: generic` for any other OpenAI-compatible endpoint \
             (vLLM, LM Studio, llama.cpp, a LiteLLM proxy)."
        ));
    }

    msg
}

/// Name of the `models` entry a "missing field" error points at, if that is
/// what `err` is. Matched on the field path rather than the surrounding
/// wording, which belongs to the `config` crate.
fn missing_provider_group(err: &str) -> Option<&str> {
    if !err.contains("missing") {
        return None;
    }
    let (_, after) = err.split_once("models.")?;
    let (group, rest) = after.split_once('.')?;
    let rest = rest.trim_end_matches(['"', '\'', '`', '.']);
    (!group.is_empty() && rest.ends_with("provider")).then_some(group)
}
