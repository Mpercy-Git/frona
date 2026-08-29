use std::collections::HashMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_aux::field_attributes::deserialize_bool_from_anything;
use surrealdb::types::SurrealValue;

const ENV_PREFIX: &str = "FRONA_";

const EXCLUDED_ENV_VARS: &[&str] = &[
    "FRONA_CONFIG",
    "FRONA_LOG_CONFIG",
    "FRONA_LOG_LEVEL",
    "FRONA_SERVER_DATA_DIR",
];

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct ServerConfig {
    #[schemars(description = "Port the server listens on.")]
    pub port: u16,
    #[schemars(description = "Path to the static frontend build directory.")]
    pub static_dir: String,
    #[schemars(description = "Issuer URL for JWT tokens.")]
    pub issuer_url: String,
    #[schemars(description = "Maximum number of concurrent background tasks.")]
    pub max_concurrent_tasks: usize,
    #[schemars(description = "Comma-separated list of allowed CORS origins.")]
    pub cors_origins: Option<String>,
    #[schemars(description = "Public base URL for the server (used for callbacks, links).")]
    pub base_url: Option<String>,
    #[schemars(description = "Override URL for the backend API (if different from base_url).")]
    pub backend_url: Option<String>,
    #[schemars(description = "Override URL for the frontend (if different from base_url).")]
    pub frontend_url: Option<String>,
    #[schemars(
        description = "Externally-reachable URL of this server (e.g. ngrok tunnel, public domain). Used as the default callback target for inbound webhooks and external service callbacks when no per-feature override is set."
    )]
    pub external_url: Option<String>,
    #[schemars(description = "Maximum request body size in bytes.")]
    pub max_body_size_bytes: usize,
    #[schemars(
        description = "Graceful shutdown timeout in seconds. Server force-exits after this duration."
    )]
    pub shutdown_timeout_secs: u64,
    #[schemars(
        description = "Seconds to buffer SSE events after a client disconnects, allowing reconnects to receive missed events. 0 disables."
    )]
    pub sse_pending_events_secs: u64,
    #[schemars(
        description = "Server-default IANA timezone (e.g. \"America/Los_Angeles\"). Used when a user has no timezone set and no per-task override is provided. Leave empty to auto-detect from TZ env var, /etc/localtime, or fall back to UTC."
    )]
    pub timezone: String,
}

impl ServerConfig {
    pub fn public_base_url(&self) -> String {
        self.backend_url
            .as_deref()
            .or(self.base_url.as_deref())
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string()
    }

    pub fn public_frontend_url(&self) -> String {
        self.frontend_url
            .as_deref()
            .or(self.base_url.as_deref())
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string()
    }

    pub fn external_base_url(&self) -> Option<String> {
        self.external_url
            .as_deref()
            .or(self.backend_url.as_deref())
            .or(self.base_url.as_deref())
            .map(|s| s.trim_end_matches('/').to_string())
    }

    /// Always returns an openable URL (unlike `public_base_url()` which may be empty).
    pub fn external_or_local_base_url(&self) -> String {
        self.external_base_url()
            .unwrap_or_else(|| format!("http://localhost:{}", self.port))
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3001,
            static_dir: "/app/static".into(),
            issuer_url: String::new(),
            max_concurrent_tasks: 10,
            cors_origins: None,
            base_url: None,
            backend_url: None,
            frontend_url: None,
            external_url: None,
            max_body_size_bytes: 104_857_600,
            shutdown_timeout_secs: 60,
            sse_pending_events_secs: 60,
            timezone: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
#[serde(default)]
pub struct SandboxLimits {
    #[schemars(
        description = "Per-principal CPU usage limit as percentage of total system CPU. Kill sandboxed process if exceeded."
    )]
    pub max_cpu_pct: f64,
    #[schemars(
        description = "Per-principal memory usage limit as percentage of total system memory. Kill sandboxed process if exceeded."
    )]
    pub max_memory_pct: f64,
    #[schemars(
        description = "Default timeout in seconds for sandboxed execution. 0 means no timeout."
    )]
    pub timeout_secs: u64,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            max_cpu_pct: 95.0,
            max_memory_pct: 80.0,
            timeout_secs: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct SandboxConfig {
    #[schemars(
        description = "Disable filesystem sandboxing for CLI tools. Enable only if your OS does not support Landlock."
    )]
    pub disabled: bool,
    #[serde(flatten)]
    pub default_limits: SandboxLimits,
    #[schemars(
        description = "Global CPU usage limit across all sandboxed processes as percentage of total system CPU."
    )]
    pub max_total_cpu_pct: f64,
    #[schemars(
        description = "Global memory usage limit across all sandboxed processes as percentage of total system memory."
    )]
    pub max_total_memory_pct: f64,
    #[schemars(
        description = "Grant all sandbox principals outbound network access by default. Override with forbid policies."
    )]
    pub default_network_access: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            disabled: false,
            default_limits: SandboxLimits::default(),
            max_total_cpu_pct: 98.0,
            max_total_memory_pct: 90.0,
            default_network_access: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct AuthConfig {
    #[schemars(description = "Secret key for JWT signing. Change from default in production.")]
    pub encryption_secret: String,
    #[schemars(description = "Access token lifetime in seconds.")]
    pub access_token_expiry_secs: u64,
    #[schemars(description = "Refresh token lifetime in seconds.")]
    pub refresh_token_expiry_secs: u64,
    #[schemars(description = "Presigned URL expiry in seconds.")]
    pub presign_expiry_secs: u64,
    #[schemars(
        description = "Ephemeral principal token lifetime in seconds (stateless; injected into sandboxed processes)."
    )]
    pub ephemeral_token_expiry_secs: u64,
    #[schemars(
        description = "Allow anyone to sign up from the registration page. When off, only admins can add users."
    )]
    pub allow_registration: bool,
    #[schemars(description = "Consecutive failed login attempts before an account is temporarily locked. 0 disables lockout.")]
    pub max_login_attempts: u32,
    #[schemars(description = "How long an account stays locked after too many failed logins, in minutes.")]
    pub lockout_minutes: u64,
    #[schemars(description = "Lifetime of an emailed password-reset link, in minutes.")]
    pub password_reset_expiry_minutes: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            encryption_secret: "dev-secret-change-in-production".into(),
            access_token_expiry_secs: 900,
            refresh_token_expiry_secs: 604800,
            presign_expiry_secs: 86400,
            ephemeral_token_expiry_secs: 300,
            allow_registration: true,
            max_login_attempts: 5,
            lockout_minutes: 15,
            password_reset_expiry_minutes: 30,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct SsoConfig {
    #[schemars(description = "Enable SSO/OIDC authentication.")]
    pub enabled: bool,
    #[schemars(description = "OIDC authority/issuer URL (e.g. https://accounts.google.com).")]
    pub authority: Option<String>,
    #[schemars(description = "OIDC client ID.")]
    pub client_id: Option<String>,
    #[schemars(description = "OIDC client secret.")]
    pub client_secret: Option<String>,
    #[schemars(description = "OIDC scopes to request.")]
    pub scopes: String,
    #[schemars(description = "Allow verification of emails not matching known users.")]
    pub allow_unknown_email_verification: bool,
    #[schemars(description = "Client cache expiration in seconds.")]
    pub client_cache_expiration: u64,
    #[schemars(description = "Disable local (email/password) authentication when SSO is enabled.")]
    pub disable_local_auth: bool,
    #[schemars(description = "Match SSO signups to existing users by email.")]
    pub signups_match_email: bool,
}

impl Default for SsoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            authority: None,
            client_id: None,
            client_secret: None,
            scopes: "openid email".into(),
            allow_unknown_email_verification: true,
            client_cache_expiration: 0,
            disable_local_auth: false,
            signups_match_email: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct DatabaseConfig {
    #[schemars(description = "Path to the SurrealDB data directory.")]
    pub path: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: "data/db".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct BrowserConfig {
    #[schemars(description = "WebSocket URL for browserless (e.g. ws://browserless:3333).")]
    pub ws_url: String,
    #[schemars(description = "Authentication token for the browserless HTTP API.")]
    #[serde(default)]
    pub api_token: Option<String>,
    #[schemars(description = "Path to store browser profiles.")]
    pub profiles_path: String,
    #[schemars(description = "Browser connection timeout in milliseconds.")]
    pub connection_timeout_ms: u64,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            ws_url: String::new(),
            api_token: None,
            profiles_path: "/profiles".into(),
            connection_timeout_ms: 30000,
        }
    }
}

impl BrowserConfig {
    pub fn http_base_url(&self) -> String {
        self.ws_url
            .replace("ws://", "http://")
            .replace("wss://", "https://")
    }

    /// Browserless v2 requires a `token` query param on management endpoints
    /// (`/sessions`, `/kill`) even when no TOKEN env var is configured server-side.
    /// Falls back to "frona" which satisfies the schema validation.
    pub fn api_token(&self) -> &str {
        self.api_token.as_deref().unwrap_or("frona")
    }

    pub fn debugger_url_for_credential(&self, credential_id: &str) -> String {
        format!("/api/browser/debugger/{credential_id}")
    }

    pub fn profile_path(&self, handle: &crate::core::Handle, provider: &str) -> PathBuf {
        PathBuf::from(&self.profiles_path)
            .join(handle.as_ref())
            .join(provider)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct SearchConfig {
    #[schemars(description = "Search provider (searxng, tavily, or brave).")]
    pub provider: Option<String>,
    #[schemars(description = "Base URL for SearXNG instance.")]
    pub searxng_base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct StorageConfig {
    #[schemars(
        description = "Root data directory. Per-user state lives at `{data_dir}/users/{user_handle}/...`."
    )]
    pub data_dir: String,
    #[schemars(
        description = "Path to shared configuration resources (read-only, ships with the binary)."
    )]
    pub shared_config_dir: String,
    #[schemars(description = "Path for installed skills directory.")]
    pub skills_dir: String,
    #[schemars(description = "Path for system cache directory.")]
    pub cache_dir: String,
    #[schemars(
        description = "Path for your own ontologies. Loaded alongside the bundled \
        ones and trusted the same, so a file here can retype or untype pages. Need not \
        exist."
    )]
    pub ontology_dir: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: "data".into(),
            shared_config_dir: "resources".into(),
            skills_dir: "data/skills".into(),
            cache_dir: "data/system/cache".into(),
            ontology_dir: "data/ontology".into(),
        }
    }
}

impl StorageConfig {
    /// The two directories the ontology catalogue is assembled from.
    ///
    /// The bundled half is derived from `shared_config_dir` rather than configurable:
    /// it is image content, read-only, and replaced wholesale by an upgrade. In a
    /// container it is a layer, so nothing may be written there - it would vanish on
    /// restart. Anything fetched at runtime belongs in `ontology_dir`, which sits on
    /// the data volume and persists.
    ///
    /// They stay separate rather than merging into one because source attribution is
    /// assigned on first sight, and "gone because a newer image replaced it" has to
    /// remain distinguishable from "the user deleted it" - one is an upgrade, the other
    /// is intent.
    pub fn ontology_roots(&self) -> crate::memory::pkm::ontology::Roots {
        crate::memory::pkm::ontology::Roots {
            release: PathBuf::from(&self.shared_config_dir).join("ontology"),
            user: PathBuf::from(&self.ontology_dir),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct SchedulerConfig {
    #[schemars(description = "Scheduler poll interval in seconds.")]
    pub poll_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { poll_secs: 60 }
    }
}

#[derive(
    Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, surrealdb::types::SurrealValue,
)]
#[surreal(crate = "surrealdb::types")]
#[serde(default)]
pub struct RetryConfig {
    #[schemars(description = "Maximum number of retry attempts. 0 disables retry.")]
    pub max_retries: u32,
    #[schemars(description = "Initial backoff delay in milliseconds.")]
    pub initial_backoff_ms: u64,
    #[schemars(description = "Multiplier applied to backoff delay between retries.")]
    pub backoff_multiplier: f64,
    #[schemars(description = "Maximum backoff delay in milliseconds.")]
    pub max_backoff_ms: u64,
}

impl RetryConfig {
    pub fn to_backoff(&self) -> backon::ExponentialBuilder {
        backon::ExponentialBuilder::default()
            .with_max_times(self.max_retries as usize)
            .with_min_delay(std::time::Duration::from_millis(self.initial_backoff_ms))
            .with_factor(self.backoff_multiplier as f32)
            .with_max_delay(std::time::Duration::from_millis(self.max_backoff_ms))
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            initial_backoff_ms: 1_000,
            backoff_multiplier: 2.0,
            max_backoff_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct ShareConfig {
    #[schemars(description = "TTL in seconds for newly-issued Share rows (short links).")]
    pub ttl_secs: u64,
    #[schemars(description = "Interval in seconds between expired-share cleanup runs.")]
    pub cleanup_interval_secs: u64,
}

impl Default for ShareConfig {
    fn default() -> Self {
        Self {
            ttl_secs: 30 * 24 * 60 * 60,
            cleanup_interval_secs: 6 * 60 * 60,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct PushConfig {
    #[schemars(description = "VAPID public key (base64url-encoded, uncompressed P-256). Required for Web Push.")]
    pub vapid_public_key: Option<String>,
    #[schemars(description = "VAPID private key (base64url-encoded, uncompressed P-256). Required for Web Push.")]
    pub vapid_private_key: Option<String>,
    #[schemars(description = "VAPID subject — a mailto: URL or the site's HTTPS URL.")]
    pub subject: String,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            vapid_public_key: None,
            vapid_private_key: None,
            subject: "mailto:noreply@frona.local".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SmtpTls {
    /// STARTTLS upgrade on the submission port (587). The usual choice.
    #[default]
    Starttls,
    /// TLS from the first byte (465).
    Implicit,
    /// Plaintext. Only sane for a relay on localhost or a dev mail catcher.
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct MailConfig {
    #[schemars(description = "SMTP server hostname. Leave empty to disable outbound email (and with it, password reset).")]
    pub smtp_host: String,
    #[schemars(description = "SMTP server port.")]
    pub smtp_port: u16,
    #[schemars(description = "SMTP username. Leave empty for an unauthenticated relay.")]
    pub smtp_username: Option<String>,
    #[schemars(description = "SMTP password.")]
    pub smtp_password: Option<String>,
    #[schemars(description = "Transport security: starttls (587), implicit (465), or none.")]
    pub tls: SmtpTls,
    #[schemars(description = "Envelope sender address for outbound mail.")]
    pub from_address: String,
    #[schemars(description = "Display name shown alongside the sender address.")]
    pub from_name: String,
}

impl Default for MailConfig {
    fn default() -> Self {
        Self {
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_username: None,
            smtp_password: None,
            tls: SmtpTls::Starttls,
            from_address: "noreply@frona.local".into(),
            from_name: "Frona".into(),
        }
    }
}

impl MailConfig {
    /// Outbound mail is opt-in: an empty host means the feature is off, rather
    /// than a misconfiguration to fail startup over.
    pub fn is_configured(&self) -> bool {
        !self.smtp_host.trim().is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct ChannelConfig {
    #[schemars(
        description = "Default retry policy for failed channel connections. Per-channel overrides take precedence."
    )]
    pub retry: RetryConfig,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            retry: RetryConfig {
                max_retries: u32::MAX,
                initial_backoff_ms: 1_000,
                backoff_multiplier: 2.0,
                max_backoff_ms: 60_000,
            },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct CommonModelFields {
    #[schemars(description = "Model ID (without provider prefix).")]
    pub model: String,
    #[serde(default)]
    #[schemars(description = "Fallback models tried in order if the primary fails.")]
    pub fallbacks: Vec<ModelGroupConfig>,
    #[serde(default)]
    #[schemars(description = "Maximum tokens to generate per response.")]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    #[schemars(description = "Sampling temperature (0.0-2.0).")]
    pub temperature: Option<f64>,
    #[serde(default)]
    #[schemars(description = "Context window size override.")]
    pub context_window: Option<usize>,
    #[serde(default)]
    #[schemars(description = "Retry configuration for this model group.")]
    pub retry: RetryConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct AnthropicThinking {
    #[serde(rename = "type")]
    #[schemars(description = "'enabled' or 'disabled'.")]
    pub thinking_type: String,
    #[serde(default)]
    #[schemars(description = "Token budget for thinking (required when type is 'enabled').")]
    pub budget_tokens: Option<u64>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct OpenAICompatParams {
    pub top_p: Option<f64>,
    pub min_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub seed: Option<i64>,
    pub max_completion_tokens: Option<u64>,
    #[schemars(description = "Reasoning effort level (e.g. 'low', 'medium', 'high').")]
    pub reasoning_effort: Option<String>,
    pub logprobs: Option<bool>,
    pub top_logprobs: Option<u64>,
    pub stop: Option<Vec<String>>,
}

/// OpenRouter provider routing configuration.
///
/// Maps to the `provider` object in the OpenRouter API request.
/// See https://openrouter.ai/docs/guides/routing/provider-selection
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct OpenRouterProviderRouting {
    /// Ordered list of provider names to try (e.g. ["OpenAI", "Anthropic"]).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "order")]
    #[schemars(description = "Ordered provider preferences, e.g. ['OpenAI', 'Anthropic'].")]
    pub order: Option<Vec<String>>,
    /// Whether to allow fallback to other providers if preferred ones fail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Allow fallback to other providers if preferred ones fail (default true).")]
    pub allow_fallbacks: Option<bool>,
    /// Require all preferred providers to support the request parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Require all preferred providers to support the request parameters.")]
    pub require_parameters: Option<bool>,
    /// Ignore specific providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Ignore specific providers, e.g. ['Together'].")]
    pub ignore: Option<Vec<String>>,
    /// Quantization filter, e.g. ["fp8", "bf16"].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Quantization preferences, e.g. ['fp8', 'bf16'].")]
    pub quantizations: Option<Vec<String>>,
    /// Sort preference for provider selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Sort providers by: 'throughput', 'latency', 'price'.")]
    pub sort: Option<String>,
}

/// OpenRouter-specific parameters beyond the OpenAI-compatible fields.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct OpenRouterParams {
    #[serde(flatten)]
    pub compat: OpenAICompatParams,
    /// Simple routing string, e.g. "openai" or "anthropic".
    /// Sent as top-level `route` in the API request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Simple provider routing, e.g. 'openai' or 'anthropic'. Mutually exclusive with provider routing.")]
    pub route: Option<String>,
    /// Provider routing object. Sent as `provider` in the API request.
    /// Renamed to avoid collision with the `#[serde(tag = "provider")]` enum discriminant.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "provider_routing")]
    #[schemars(description = "Provider routing preferences (order, fallbacks, etc.). See https://openrouter.ai/docs/guides/routing/provider-selection")]
    pub provider_routing: Option<OpenRouterProviderRouting>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GeminiThinkingConfig {
    pub thinking_budget: u64,
    pub include_thoughts: Option<bool>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct AnthropicParams {
    pub thinking: Option<AnthropicThinking>,
    pub top_p: Option<f64>,
    pub top_k: Option<u64>,
    pub stop_sequences: Option<Vec<String>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct OllamaParams {
    pub think: Option<bool>,
    pub num_ctx: Option<u64>,
    pub num_predict: Option<u64>,
    pub num_batch: Option<u64>,
    pub num_keep: Option<i64>,
    pub num_thread: Option<u64>,
    pub num_gpu: Option<u64>,
    pub top_k: Option<u64>,
    pub top_p: Option<f64>,
    pub min_p: Option<f64>,
    pub repeat_penalty: Option<f64>,
    pub repeat_last_n: Option<i64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub mirostat: Option<u64>,
    pub mirostat_eta: Option<f64>,
    pub mirostat_tau: Option<f64>,
    pub tfs_z: Option<f64>,
    pub seed: Option<i64>,
    pub stop: Option<Vec<String>>,
    pub use_mmap: Option<bool>,
    pub use_mlock: Option<bool>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct GeminiParams {
    pub thinking_config: Option<GeminiThinkingConfig>,
    pub top_p: Option<f64>,
    pub top_k: Option<u64>,
    pub stop_sequences: Option<Vec<String>>,
    pub candidate_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiApi {
    #[default]
    ChatCompletions,
    Responses,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "provider")]
pub enum ProviderModel {
    #[serde(rename = "anthropic")]
    Anthropic {
        #[serde(flatten)]
        params: AnthropicParams,
    },
    #[serde(rename = "ollama")]
    Ollama {
        #[serde(flatten)]
        params: OllamaParams,
    },
    #[serde(rename = "openai")]
    OpenAI {
        api: Option<OpenAiApi>,
        #[serde(flatten)]
        params: OpenAICompatParams,
    },
    #[serde(rename = "groq")]
    Groq {
        #[serde(flatten)]
        params: OpenAICompatParams,
    },
    #[serde(rename = "openrouter")]
    OpenRouter {
        #[serde(flatten)]
        params: OpenRouterParams,
    },
    #[serde(rename = "deepseek")]
    DeepSeek {
        #[serde(flatten)]
        params: OpenAICompatParams,
    },
    #[serde(rename = "xai")]
    XAI {
        #[serde(flatten)]
        params: OpenAICompatParams,
    },
    #[serde(rename = "together")]
    Together {
        #[serde(flatten)]
        params: OpenAICompatParams,
    },
    #[serde(rename = "hyperbolic")]
    Hyperbolic {
        #[serde(flatten)]
        params: OpenAICompatParams,
    },
    #[serde(rename = "gemini")]
    Gemini {
        #[serde(flatten)]
        params: GeminiParams,
    },
    #[serde(rename = "generic")]
    #[default]
    Generic,
    #[serde(skip)]
    #[schemars(skip)]
    Custom { name: String },
}

impl From<&str> for ProviderModel {
    fn from(name: &str) -> Self {
        Self::from_name(name)
    }
}

impl From<String> for ProviderModel {
    fn from(name: String) -> Self {
        Self::from_name(&name)
    }
}

impl ProviderModel {
    pub fn from_name(name: &str) -> Self {
        match name {
            "anthropic" => Self::Anthropic {
                params: Default::default(),
            },
            "ollama" => Self::Ollama {
                params: Default::default(),
            },
            "openai" => Self::OpenAI {
                api: None,
                params: Default::default(),
            },
            "groq" => Self::Groq {
                params: Default::default(),
            },
            "openrouter" => Self::OpenRouter {
                params: Default::default(),
            },
            "deepseek" => Self::DeepSeek {
                params: Default::default(),
            },
            "xai" => Self::XAI {
                params: Default::default(),
            },
            "together" => Self::Together {
                params: Default::default(),
            },
            "hyperbolic" => Self::Hyperbolic {
                params: Default::default(),
            },
            "gemini" => Self::Gemini {
                params: Default::default(),
            },
            "generic" => Self::Generic,
            name => Self::Custom {
                name: name.to_string(),
            },
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Anthropic { .. } => "anthropic",
            Self::Ollama { .. } => "ollama",
            Self::OpenAI { .. } => "openai",
            Self::Groq { .. } => "groq",
            Self::OpenRouter { .. } => "openrouter",
            Self::DeepSeek { .. } => "deepseek",
            Self::XAI { .. } => "xai",
            Self::Together { .. } => "together",
            Self::Hyperbolic { .. } => "hyperbolic",
            Self::Gemini { .. } => "gemini",
            Self::Generic => "generic",
            Self::Custom { name } => name,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct ModelGroupConfig {
    #[serde(flatten)]
    pub common: CommonModelFields,
    #[serde(flatten)]
    pub provider: ProviderModel,
}

impl ModelGroupConfig {
    pub fn common(&self) -> &CommonModelFields {
        &self.common
    }

    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }

    /// Extract provider-specific params as JSON for Rig's additional_params.
    /// Serializes the whole config, strips common fields and the provider tag,
    /// returning only provider-specific params. Returns None if empty.
    /// Also renames `provider_routing` to `provider` for OpenRouter API compat.
    pub fn additional_params(&self) -> Option<serde_json::Value> {
        const COMMON_KEYS: &[&str] = &[
            "provider", "model", "fallbacks", "max_tokens",
            "temperature", "context_window", "retry",
        ];

        let mut map = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => return None,
        };

        for key in COMMON_KEYS {
            map.remove(*key);
        }

        // Rename `provider_routing` -> `provider` for the OpenRouter API.
        // We use `provider_routing` in the config to avoid colliding with
        // the `#[serde(tag = "provider")]` enum discriminant.
        if let Some(routing) = map.remove("provider_routing") {
            map.insert("provider".to_string(), routing);
        }

        map.retain(|_, v| !v.is_null());

        if map.is_empty() { None } else { Some(serde_json::Value::Object(map)) }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct ModelProviderConfig {
    #[schemars(description = "API key for this provider. Supports ${ENV_VAR} references.")]
    pub api_key: Option<String>,
    #[schemars(description = "Custom base URL for this provider's API.")]
    pub base_url: Option<String>,
    #[serde(
        default = "serde_aux::prelude::bool_true",
        deserialize_with = "deserialize_bool_from_anything"
    )]
    #[schemars(description = "Whether this provider is enabled.")]
    pub enabled: bool,
}

impl Default for ModelProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct InferenceConfig {
    #[schemars(description = "Maximum number of tool-use turns per inference loop.")]
    pub max_tool_turns: usize,
    #[schemars(description = "Default max tokens when not specified by model group.")]
    pub default_max_tokens: u64,
    #[schemars(description = "Percentage of context window usage that triggers compaction.")]
    pub compaction_trigger_pct: usize,
    #[schemars(description = "Percentage of history to keep after truncation.")]
    pub history_truncation_pct: usize,
    #[schemars(description = "Per-tool-call execution timeout in seconds. A hung tool (e.g. an unresponsive MCP server) fails after this instead of stalling the message forever. 0 disables the timeout.")]
    pub tool_timeout_secs: u64,
    #[schemars(description = "Model ids to force as vision-capable, overriding the catalog. Matches the model id (e.g. \"deepseek-v4-flash\"), a \"provider/model\" pair, or a vendor-prefixed suffix.")]
    pub vision_models: Vec<String>,
    #[schemars(description = "Model ids to force as text-only (no image input), overriding the catalog. Same matching as vision_models. Wins over the catalog and over vision_models.")]
    pub text_only_models: Vec<String>,
    #[schemars(description = "When a model's image support is unknown (absent from the catalog and both override lists), treat it as text-only so images are transcribed or stripped instead of risking a provider 404. Default false.")]
    pub transcribe_when_vision_unknown: bool,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            max_tool_turns: 200,
            default_max_tokens: 8192,
            compaction_trigger_pct: 80,
            history_truncation_pct: 90,
            tool_timeout_secs: 600,
            vision_models: Vec::new(),
            text_only_models: Vec::new(),
            transcribe_when_vision_unknown: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct VoiceConfig {
    #[schemars(description = "Voice provider (twilio, plivo, or none).")]
    pub provider: Option<String>,
    #[schemars(description = "Twilio account SID.")]
    pub twilio_account_sid: Option<String>,
    #[schemars(description = "Twilio auth token.")]
    pub twilio_auth_token: Option<String>,
    #[schemars(description = "Twilio phone number to call from.")]
    pub twilio_from_number: Option<String>,
    #[schemars(description = "Twilio voice ID for text-to-speech.")]
    pub twilio_voice_id: Option<String>,
    #[schemars(description = "Twilio speech recognition model.")]
    pub twilio_speech_model: Option<String>,
    #[schemars(description = "TTS provider for ConversationRelay (e.g. elevenlabs, polly). Defaults to polly when not set.")]
    pub twilio_tts_provider: Option<String>,
    #[schemars(description = "How readily the agent yields when the caller starts speaking: low, medium, or high. Higher cuts the agent off sooner but false-triggers on background noise. Defaults to medium.")]
    pub twilio_interrupt_sensitivity: Option<String>,
    #[schemars(description = "Plivo auth ID.")]
    pub plivo_auth_id: Option<String>,
    #[schemars(description = "Plivo auth token.")]
    pub plivo_auth_token: Option<String>,
    #[schemars(description = "Plivo phone number to call from.")]
    pub plivo_from_number: Option<String>,
    #[schemars(description = "Public-facing base URL for voice callbacks. Overrides server.base_url for voice only.")]
    pub callback_base_url: Option<String>,
    #[schemars(description = "Enable inbound call answering. Requires the voice provider to POST to the inbound webhook.")]
    pub inbound_enabled: bool,
    #[schemars(description = "Server-level default greeting spoken when an inbound call connects, used when the owning user has not set their own via /api/voice/inbound-settings.")]
    pub inbound_welcome_greeting: Option<String>,
    #[schemars(description = "Enable silence filling during agent processing — sends periodic filler phrases while the agent is thinking. Only applies when the remote party's number matches a registered user (a user calling in, or the agent calling one of its users); calls with third parties are unaffected, as the agent narrates its own progress there per the active-call prompt.")]
    pub silence_fill_enabled: bool,
    #[schemars(description = "Seconds of silence before the first filler phrase is sent. Defaults to 2 — longer feels like dead air on a phone call.")]
    #[serde(default = "default_silence_fill_initial_delay_secs")]
    pub silence_fill_initial_delay_secs: u64,
    #[schemars(description = "Seconds between successive filler phrases. Defaults to 7.")]
    #[serde(default = "default_silence_fill_interval_secs")]
    pub silence_fill_interval_secs: u64,
    #[schemars(description = "Filler phrases spoken to the caller while the agent is processing. Each interval advances to the next phrase in order (rotating). If empty, uses built-in defaults.")]
    pub silence_fill_phrases: Vec<String>,
}

fn default_silence_fill_initial_delay_secs() -> u64 {
    2
}

fn default_silence_fill_interval_secs() -> u64 {
    7
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct VaultConfig {
    #[schemars(description = "1Password service account token (for the `op` CLI).")]
    pub onepassword_service_account_token: Option<String>,
    #[schemars(description = "1Password vault ID.")]
    pub onepassword_vault_id: Option<String>,
    #[schemars(description = "Bitwarden CLI client ID (personal API key).")]
    pub bitwarden_client_id: Option<String>,
    #[schemars(description = "Bitwarden CLI client secret (personal API key).")]
    pub bitwarden_client_secret: Option<String>,
    #[schemars(description = "Bitwarden master password (for vault unlock).")]
    pub bitwarden_master_password: Option<String>,
    #[schemars(
        description = "Bitwarden server URL (for self-hosted instances, leave empty for cloud)."
    )]
    pub bitwarden_server_url: Option<String>,
    #[schemars(description = "HashiCorp Vault server address.")]
    pub hashicorp_address: Option<String>,
    #[schemars(description = "HashiCorp Vault access token.")]
    pub hashicorp_token: Option<String>,
    #[schemars(description = "HashiCorp Vault secrets mount path.")]
    pub hashicorp_mount: Option<String>,
    #[schemars(description = "Path to KeePass database file.")]
    pub keepass_path: Option<String>,
    #[schemars(description = "KeePass database password.")]
    pub keepass_password: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct AppConfig {
    #[schemars(description = "Start of port range for managed apps.")]
    pub port_range_start: u16,
    #[schemars(description = "End of port range for managed apps.")]
    pub port_range_end: u16,
    #[schemars(description = "Health check timeout in seconds.")]
    pub health_check_timeout_secs: u64,
    #[schemars(description = "Maximum process restart attempts before marking as failed.")]
    pub max_restart_attempts: u32,
    #[schemars(description = "Seconds of inactivity before an app is auto-hibernated.")]
    pub hibernate_after_secs: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port_range_start: 4000,
            port_range_end: 4100,
            health_check_timeout_secs: 30,
            max_restart_attempts: 2,
            hibernate_after_secs: 259200,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct CacheConfig {
    #[schemars(description = "TTL in seconds for cached entities (agents, users).")]
    pub entity_ttl_secs: u64,
    #[schemars(description = "Maximum number of cached entities.")]
    pub entity_max_capacity: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            entity_ttl_secs: 300,
            entity_max_capacity: 1000,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct McpConfig {
    #[schemars(description = "Whether MCP server support is enabled.")]
    pub enabled: bool,
    #[schemars(
        description = "Path for shared package caches (npm, uv). Defaults to `{data_dir}/system/mcp-cache`."
    )]
    #[serde(default)]
    pub cache_path: Option<String>,
    #[schemars(description = "Maximum number of MCP servers a user may have installed.")]
    pub max_servers_per_user: u32,
    #[schemars(
        description = "Seconds to wait for an MCP server's initialize handshake before failing."
    )]
    pub startup_timeout_secs: u64,
    #[schemars(description = "Interval in seconds between MCP server liveness checks.")]
    pub health_check_interval_secs: u64,
    #[schemars(
        description = "Maximum process restart attempts before marking an MCP server as failed."
    )]
    pub max_restart_attempts: u32,
    #[schemars(description = "Default transport for new MCP servers: 'stdio' or 'http'.")]
    pub default_transport: String,
    #[schemars(description = "Start of the port range for local HTTP MCP servers.")]
    pub port_range_start: u16,
    #[schemars(description = "End of the port range for local HTTP MCP servers (exclusive).")]
    pub port_range_end: u16,
    #[schemars(
        description = "When true, expose MCP tools via the mcpctl CLI bridge instead of individual tool definitions. Reduces LLM context token usage."
    )]
    pub bridge_mode: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_path: None,
            max_servers_per_user: 32,
            startup_timeout_secs: 30,
            health_check_interval_secs: 10,
            max_restart_attempts: 3,
            default_transport: "stdio".into(),
            port_range_start: 4100,
            port_range_end: 4200,
            bridge_mode: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, JsonSchema)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub sandbox: SandboxConfig,
    pub auth: AuthConfig,
    pub sso: SsoConfig,
    pub database: DatabaseConfig,
    pub browser: Option<BrowserConfig>,
    pub search: SearchConfig,
    pub vault: VaultConfig,
    pub storage: StorageConfig,
    pub scheduler: SchedulerConfig,
    pub inference: InferenceConfig,
    pub memory: MemoryConfig,
    pub voice: VoiceConfig,
    pub app: AppConfig,
    pub cache: CacheConfig,
    pub mcp: McpConfig,
    #[serde(default)]
    pub channel: ChannelConfig,
    #[serde(default)]
    pub share: ShareConfig,
    #[serde(default)]
    pub push: PushConfig,
    #[serde(default)]
    pub mail: MailConfig,
    #[serde(default)]
    pub signal: SignalConfig,
    #[serde(default)]
    pub models: HashMap<String, ModelGroupConfig>,
    #[serde(default)]
    pub providers: HashMap<String, ModelProviderConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct SignalConfig {
    #[schemars(description = "Maximum number of pending signal watches per user.")]
    pub max_pending_per_user: usize,
    #[schemars(
        description = "Default safety cap on the number of candidates a one-shot watch can be evaluated against before auto-failing. Sweep cadence is driven by scheduler.poll_secs."
    )]
    pub default_max_evaluations: u32,
    #[schemars(
        description = "Default safety cap on the number of fires a continuous-mode watch can absorb before auto-completing. Higher than the one-shot default because continuous watches stream over time."
    )]
    pub default_max_continuous_evaluations: u32,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            max_pending_per_user: 50,
            default_max_evaluations: 50,
            default_max_continuous_evaluations: 1_000,
        }
    }
}

/// Which memory backend runs. `basic` rolls loose
/// `memory_entry` rows into compacted summaries; `pkm` builds a knowledge base
/// of pages from background consolidation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryBackend {
    #[default]
    Basic,
    Pkm,
}

/// Memory subsystem configuration. Flat (single level under `memory`) so every
/// knob is reachable via `FRONA_MEMORY_*` env overrides, not just YAML.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct MemoryConfig {
    #[schemars(
        description = "Which memory backend to run: `basic` or `pkm`. Unset (null) \
        resolves to `basic` at boot; the setup wizard writes `pkm` for fresh installs and \
        existing installs opt in explicitly, so upgrades stay on `basic` unless changed."
    )]
    pub backend: Option<MemoryBackend>,
    #[schemars(
        description = "Model group for memory background work (basic compaction / pkm consolidation). Falls back to `primary` if undefined."
    )]
    pub model_group: String,
    #[schemars(description = "basic: skip user/agent memory compaction below this many tokens.")]
    pub basic_compaction_token_threshold: usize,
    #[schemars(
        description = "basic: interval in seconds between user/agent memory compaction runs."
    )]
    pub basic_compaction_secs: u64,
    #[schemars(description = "basic: interval in seconds between space memory compaction runs.")]
    pub basic_space_compaction_secs: u64,
    #[schemars(description = "pkm: max hits returned by `memory_search`.")]
    pub pkm_search_top_k: i64,
    #[schemars(description = "pkm: recency-decay half-life (seconds) for short memory.")]
    pub pkm_short_memory_half_life_secs: u64,
    #[schemars(description = "pkm: drop short memory once its decay score falls below this.")]
    pub pkm_short_memory_demote_threshold: f32,
    #[schemars(description = "pkm: max short-memory lines injected into `<short_memory>`.")]
    pub pkm_short_memory_top_n: usize,
    #[schemars(description = "pkm: token budget for the `<short_memory>` block.")]
    pub pkm_short_memory_token_cap: usize,
    #[schemars(
        description = "pkm: token budget for the `<available_playbooks>` index; playbooks past the cap (lowest use_count first) are dropped."
    )]
    pub pkm_playbook_index_token_cap: usize,
    #[schemars(
        description = "pkm: how often (seconds) the consolidation sweep scans for idle chats."
    )]
    pub pkm_consolidate_secs: u64,
    #[schemars(
        description = "pkm: how long (seconds) a chat must be quiet before it's consolidated."
    )]
    pub pkm_consolidate_idle_secs: u64,
    #[schemars(
        description = "pkm: how many consolidation model calls run at once — chats being \
        mined by the sweep, and pages being authored. Bounded so a first run over a long \
        history does not open one request per chat/page simultaneously."
    )]
    pub pkm_consolidation_concurrency: usize,
    #[schemars(
        description = "pkm classify and resolve: maximum exploration-tool turns per structured conversation."
    )]
    pub pkm_consolidation_max_tool_turns: usize,
    #[schemars(
        description = "pkm classify, resolve, and reconcile: maximum structured submission attempts per conversation."
    )]
    pub pkm_consolidation_max_submissions: usize,
    #[schemars(
        description = "pkm playbook resolve and author: maximum exploration-tool turns per structured conversation."
    )]
    pub pkm_playbook_max_tool_turns: usize,
    #[schemars(
        description = "pkm playbook resolve and author: maximum structured submission attempts per conversation."
    )]
    pub pkm_playbook_max_submissions: usize,
    #[schemars(
        description = "pkm: maximum estimated transcript tokens sent to extract in one request."
    )]
    pub pkm_extract_max_tokens: usize,
    #[schemars(description = "pkm: maximum messages consumed by one extract request.")]
    pub pkm_extract_max_messages: usize,
    #[schemars(
        description = "pkm extract: number of same-chat Agent messages searched backward for successful tool evidence supporting an Agent-sourced memory."
    )]
    pub pkm_extract_agent_evidence_lookback_messages: usize,
    #[schemars(
        description = "pkm extract: token cap returned by each scoped tool-evidence search or read."
    )]
    pub pkm_extract_agent_evidence_result_token_cap: usize,
    #[schemars(
        description = "pkm: how many times a consolidation stage may fail before its pass \
        is abandoned. Attempts count the CURRENT stage and reset when the pass advances, \
        so a long pass that hiccups at several stages is not dropped for making progress."
    )]
    pub pkm_consolidation_max_attempts: u32,
    #[schemars(
        description = "pkm adjudication: maximum model submission attempts for each \
        adjudication batch, including the initial submission and guardrail revisions."
    )]
    pub pkm_adjudication_max_attempts_per_batch: usize,
    #[schemars(
        description = "pkm: fatal post-extraction checkpoint resets allowed before the pass is marked Failed. With 2, the first fatal failure restarts at Classify and the second fails terminally."
    )]
    pub pkm_consolidation_checkpoint_failure_cap: u32,
    #[schemars(
        description = "pkm: base backoff (seconds) between retries of a failed \
        consolidation pass; doubles per attempt. Retries are quantised by the sweep tick, \
        so a value below `pkm_consolidate_secs` buys nothing."
    )]
    pub pkm_consolidation_retry_base_secs: u64,
    #[schemars(
        description = "pkm: how many finished consolidation passes to keep per user as a \
        log. Older ones are dropped by the cleanup stage."
    )]
    pub pkm_consolidation_keep_records: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: None,
            model_group: "memory".into(),
            basic_compaction_token_threshold: 3_000,
            basic_compaction_secs: 7200,
            basic_space_compaction_secs: 3600,
            pkm_search_top_k: 8,
            pkm_short_memory_half_life_secs: 14 * 24 * 3600,
            pkm_short_memory_demote_threshold: 0.1,
            pkm_short_memory_top_n: 16,
            pkm_short_memory_token_cap: 3_000,
            pkm_playbook_index_token_cap: 1_500,
            pkm_consolidate_secs: 60,
            pkm_consolidate_idle_secs: 300,
            pkm_consolidation_concurrency: 4,
            pkm_consolidation_max_tool_turns: 8,
            pkm_consolidation_max_submissions: 8,
            pkm_playbook_max_tool_turns: 20,
            pkm_playbook_max_submissions: 20,
            pkm_extract_max_tokens: 10_000,
            pkm_extract_max_messages: 300,
            pkm_extract_agent_evidence_lookback_messages: 10,
            pkm_extract_agent_evidence_result_token_cap: 4_000,
            pkm_consolidation_max_attempts: 3,
            pkm_adjudication_max_attempts_per_batch: 40,
            pkm_consolidation_checkpoint_failure_cap: 2,
            pkm_consolidation_retry_base_secs: 120,
            pkm_consolidation_keep_records: 20,
        }
    }
}

pub struct LoadedConfig {
    pub config: Config,
    pub models: Option<crate::inference::config::ModelRegistryConfig>,
}

/// Builds the effective `Config` the same way the process does at startup:
/// defaults, then the on-disk YAML (if any), then `FRONA_*` env vars layered
/// on top (env always wins). Shared by `Config::load()` and the `/api/config`
/// handlers so a value set via `FRONA_BROWSER_WS_URL` (etc.) can never look
/// different — and untested — in the settings UI than what's actually running.
pub fn build_effective_config(yaml_content: Option<&str>) -> Config {
    let data_dir = std::env::var("FRONA_SERVER_DATA_DIR")
        .unwrap_or_else(|_| "data".into());

    let mut builder = config::Config::builder()
        .set_default("database.path", format!("{data_dir}/db")).unwrap()
        .set_default("storage.data_dir", data_dir.clone()).unwrap()
        .set_default("storage.skills_dir", format!("{data_dir}/skills")).unwrap()
        .set_default("storage.cache_dir", format!("{data_dir}/system/cache")).unwrap()
        .set_default("storage.ontology_dir", format!("{data_dir}/ontology")).unwrap();

    if let Some(content) = yaml_content {
        let expanded = expand_env_vars(content);
        builder = builder.add_source(
            config::File::from_str(&expanded, config::FileFormat::Yaml),
        );
    }

    // FRONA_BROWSER_WS_URL → browser__ws_url → browser.ws_url
    let frona_env: HashMap<String, String> = std::env::vars()
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

    let built = builder.build().expect("Failed to build config");

    built.try_deserialize().expect("Failed to deserialize config")
}

impl Config {
    pub fn load() -> LoadedConfig {
        let config_path = config_file_path();

        let yaml_content = std::fs::read_to_string(&config_path).ok();

        let mut config = build_effective_config(yaml_content.as_deref());

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
            tracing::info!(path = %config_path, "Loaded config from YAML");
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

        LoadedConfig { config, models }
    }
}

/// Paths to sensitive config fields. Used for both log redaction and API response masking.
pub const SENSITIVE_PATHS: &[&[&str]] = &[
    &["auth", "encryption_secret"],
    &["sso", "client_secret"],
    &["voice", "twilio_account_sid"],
    &["voice", "twilio_auth_token"],
    &["voice", "plivo_auth_id"],
    &["voice", "plivo_auth_token"],
    &["vault", "onepassword_service_account_token"],
    &["vault", "bitwarden_client_secret"],
    &["vault", "bitwarden_master_password"],
    &["vault", "hashicorp_token"],
    &["vault", "keepass_password"],
    &["mail", "smtp_password"],
];

/// Provider fields that are sensitive (applied to each provider in the map).
pub const SENSITIVE_PROVIDER_FIELDS: &[&str] = &["api_key"];

/// Panics on explicit invalid config (fail-fast at startup, not silently mis-schedule).
pub fn resolve_server_timezone(server: &mut ServerConfig) {
    if !server.timezone.is_empty() {
        if server.timezone.parse::<chrono_tz::Tz>().is_err() {
            panic!(
                "Invalid server.timezone '{}' — must be an IANA timezone (e.g. 'America/Los_Angeles', 'Asia/Tokyo', 'UTC')",
                server.timezone
            );
        }
        tracing::info!(timezone = %server.timezone, "Server timezone resolved (explicit)");
        return;
    }

    // iana_time_zone ignores TZ when /etc/timezone disagrees → "Etc/UTC" in containers.
    if let Ok(tz_env) = std::env::var("TZ")
        && !tz_env.is_empty()
        && tz_env.parse::<chrono_tz::Tz>().is_ok()
    {
        tracing::info!(timezone = %tz_env, "Server timezone resolved (TZ env var)");
        server.timezone = tz_env;
        return;
    }

    let detected = iana_time_zone::get_timezone().ok();
    let resolved = detected
        .filter(|tz| tz.parse::<chrono_tz::Tz>().is_ok())
        .unwrap_or_else(|| "UTC".to_string());
    tracing::info!(timezone = %resolved, "Server timezone resolved (auto-detected)");
    server.timezone = resolved;
}

pub fn config_file_path() -> String {
    let data_dir = std::env::var("FRONA_SERVER_DATA_DIR").unwrap_or_else(|_| "data".into());
    std::env::var("FRONA_CONFIG").unwrap_or_else(|_| format!("{data_dir}/config.yaml"))
}

/// Redact sensitive fields in a config JSON value for logging (replaces with "[redacted]").
pub fn redact_config_for_log(value: &mut serde_json::Value) {
    for path in SENSITIVE_PATHS {
        redact(value, path);
    }
    if let Some(providers) = value.get_mut("providers").and_then(|p| p.as_object_mut()) {
        for provider in providers.values_mut() {
            for field in SENSITIVE_PROVIDER_FIELDS {
                redact(provider, &[field]);
            }
        }
    }
}

const DEFAULT_ENCRYPTION_SECRET: &str = "dev-secret-change-in-production";

/// Redact sensitive fields for API responses: replaces with `{"is_set": true/false}`.
pub fn redact_config_for_api(value: &mut serde_json::Value) {
    let has_default_secret = value
        .pointer("/auth/encryption_secret")
        .and_then(|v| v.as_str())
        .is_some_and(|s| s == DEFAULT_ENCRYPTION_SECRET);

    for path in SENSITIVE_PATHS {
        redact_as_is_set(value, path);
    }
    if let Some(providers) = value.get_mut("providers").and_then(|p| p.as_object_mut()) {
        for provider in providers.values_mut() {
            for field in SENSITIVE_PROVIDER_FIELDS {
                redact_as_is_set(provider, &[field]);
            }
        }
    }

    if has_default_secret && let Some(auth) = value.get_mut("auth").and_then(|a| a.as_object_mut())
    {
        auth.insert(
            "encryption_secret".into(),
            serde_json::json!({ "is_set": false }),
        );
    }
}

fn redact_as_is_set(value: &mut serde_json::Value, path: &[&str]) {
    match path {
        [] => {}
        [key] => {
            if let Some(v) = value.get_mut(*key) {
                let is_set = match v {
                    serde_json::Value::Null => false,
                    serde_json::Value::String(s) => !s.is_empty(),
                    _ => true,
                };
                *v = serde_json::json!({ "is_set": is_set });
            }
        }
        [key, rest @ ..] => {
            if let Some(child) = value.get_mut(*key) {
                redact_as_is_set(child, rest);
            }
        }
    }
}

fn redact(value: &mut serde_json::Value, path: &[&str]) {
    match path {
        [] => {}
        [key] => {
            if let Some(v) = value.get_mut(*key)
                && !v.is_null()
            {
                *v = serde_json::Value::String("[redacted]".into());
            }
        }
        [key, rest @ ..] => {
            if let Some(child) = value.get_mut(*key) {
                redact(child, rest);
            }
        }
    }
}

/// Recursively remove fields that match the default `Config` values,
/// keeping config.yaml minimal with only user-changed values.
pub fn strip_defaults(value: &mut serde_json::Value) {
    let defaults = serde_json::to_value(Config::default()).unwrap_or_default();
    strip_defaults_recursive(value, &defaults);

    strip_map_entry_defaults::<ModelProviderConfig>(value, "providers");
    strip_map_entry_defaults::<ModelGroupConfig>(value, "models");
}

fn strip_map_entry_defaults<T: Default + serde::Serialize>(
    value: &mut serde_json::Value,
    key: &str,
) {
    let Some(map) = value.get_mut(key).and_then(|v| v.as_object_mut()) else {
        return;
    };
    let entry_defaults = serde_json::to_value(T::default()).unwrap_or_default();
    let keys: Vec<String> = map.keys().cloned().collect();
    for k in keys {
        if let Some(entry) = map.get_mut(&k) {
            strip_defaults_recursive(entry, &entry_defaults);
            if entry.as_object().is_some_and(|o| o.is_empty()) {
                map.remove(&k);
            }
        }
    }
    if map.is_empty() {
        value.as_object_mut().unwrap().remove(key);
    }
}

fn strip_defaults_recursive(value: &mut serde_json::Value, defaults: &serde_json::Value) {
    let (Some(obj), Some(def_obj)) = (value.as_object_mut(), defaults.as_object()) else {
        return;
    };

    let keys: Vec<String> = obj.keys().cloned().collect();
    for key in keys {
        let Some(def_val) = def_obj.get(&key) else {
            continue;
        };
        let Some(val) = obj.get_mut(&key) else {
            continue;
        };

        if val.is_object() && def_val.is_object() {
            strip_defaults_recursive(val, def_val);
            if val.as_object().is_some_and(|o| o.is_empty()) {
                obj.remove(&key);
            }
        } else if values_equal(val, def_val) {
            obj.remove(&key);
        }
    }
}

fn values_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => a == b,
    }
}

/// Strip defaults from `value` and persist to `path`.
/// If all values are defaults, deletes the file instead.
pub fn persist_config(value: &mut serde_json::Value, path: &str) -> Result<(), String> {
    strip_defaults(value);

    if value.as_object().is_some_and(|o| o.is_empty()) {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }

    let json_str =
        serde_json::to_string(value).map_err(|e| format!("Failed to serialize config: {e}"))?;
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&json_str)
        .map_err(|e| format!("Failed to convert config to YAML: {e}"))?;
    let yaml_str =
        serde_yaml::to_string(&yaml_val).map_err(|e| format!("Failed to serialize config: {e}"))?;

    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    std::fs::write(path, &yaml_str).map_err(|e| format!("Failed to write config file: {e}"))
}

/// Deep-merge `patch` into `base`.
/// - Objects: recursive merge
/// - `null` values: remove the key
/// - Values matching `{"is_set": ...}` shape: skip (redaction markers from GET)
/// - All other values: overwrite
pub fn deep_merge(base: &mut serde_json::Value, patch: serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                if value.is_null() {
                    base_map.remove(&key);
                } else if value.is_object()
                    && value
                        .as_object()
                        .is_some_and(|o| o.contains_key("is_set") && o.len() == 1)
                {
                } else if let Some(existing) = base_map.get_mut(&key) {
                    if existing.is_object() && value.is_object() {
                        deep_merge(existing, value);
                    } else {
                        *existing = value;
                    }
                } else {
                    base_map.insert(key, value);
                }
            }
        }
        (base, patch) => {
            *base = patch;
        }
    }
}

pub fn expand_env_vars(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next();
            let mut var_name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                var_name.push(c);
            }
            if let Ok(val) = std::env::var(&var_name) {
                result.push_str(&val);
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        unsafe { std::env::set_var("FRONA_BROWSER_WS_URL", "ws://custom:9999") };
        let loaded = Config::load();
        assert_eq!(
            loaded.config.browser.as_ref().unwrap().ws_url,
            "ws://custom:9999"
        );
        unsafe { std::env::remove_var("FRONA_BROWSER_WS_URL") };
    }

    #[test]
    fn env_var_overrides_server_port() {
        unsafe { std::env::set_var("FRONA_SERVER_PORT", "9999") };
        let loaded = Config::load();
        assert_eq!(loaded.config.server.port, 9999);
        unsafe { std::env::remove_var("FRONA_SERVER_PORT") };
    }

    #[test]
    fn build_effective_config_env_overrides_yaml_browser_ws_url() {
        // Regression test: GET/PUT /api/config used to read config.yaml
        // verbatim, so a value pinned by FRONA_BROWSER_WS_URL (which always
        // wins at real startup) could show — and test successfully — as a
        // different, inert value from config.yaml in the settings UI.
        unsafe { std::env::set_var("FRONA_BROWSER_WS_URL", "ws://from-env:3333") };
        let yaml = "browser:\n  ws_url: ws://from-yaml:3333\n";
        let config = build_effective_config(Some(yaml));
        assert_eq!(config.browser.unwrap().ws_url, "ws://from-env:3333");
        unsafe { std::env::remove_var("FRONA_BROWSER_WS_URL") };
    }

    #[test]
    fn env_var_overrides_database_path() {
        unsafe { std::env::set_var("FRONA_DATABASE_PATH", "/tmp/testdb") };
        let loaded = Config::load();
        assert_eq!(loaded.config.database.path, "/tmp/testdb");
        unsafe { std::env::remove_var("FRONA_DATABASE_PATH") };
    }

    #[test]
    fn env_var_overrides_sso_enabled() {
        unsafe { std::env::set_var("FRONA_SSO_ENABLED", "true") };
        let loaded = Config::load();
        assert!(loaded.config.sso.enabled);
        unsafe { std::env::remove_var("FRONA_SSO_ENABLED") };
    }

    #[test]
    fn env_var_overrides_auth_allow_registration() {
        unsafe { std::env::set_var("FRONA_AUTH_ALLOW_REGISTRATION", "false") };
        let loaded = Config::load();
        assert!(!loaded.config.auth.allow_registration);
        unsafe { std::env::remove_var("FRONA_AUTH_ALLOW_REGISTRATION") };
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
                    "openai": { "api_key": "sk-123" },
                },
            })
        );
    }

    #[test]
    fn strip_defaults_removes_providers_key_when_all_default() {
        let mut value = serde_json::json!({
            "providers": {
                "anthropic": { "base_url": null, "enabled": true },
            },
        });
        strip_defaults(&mut value);
        assert_eq!(value, serde_json::json!({}));
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
    fn persist_config_writes_only_non_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        let mut value = serde_json::json!({
            "server": { "port": 8080, "static_dir": "/app/static" },
        });
        persist_config(&mut value, path_str).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_yaml::from_str(&written).unwrap();
        assert_eq!(parsed, serde_json::json!({ "server": { "port": 8080 } }));
    }

    #[test]
    fn persist_config_deletes_file_when_all_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        std::fs::write(&path, "server:\n  port: 3001\n").unwrap();
        assert!(path.exists());

        let mut value = serde_json::to_value(Config::default()).unwrap();
        persist_config(&mut value, path_str).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn persist_config_noop_when_no_file_and_all_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let path_str = path.to_str().unwrap();

        assert!(!path.exists());

        let mut value = serde_json::to_value(Config::default()).unwrap();
        persist_config(&mut value, path_str).unwrap();

        assert!(!path.exists());
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
        assert!(!rendered.contains("hunter2-smtp"), "API response leaked the SMTP password");
        assert_eq!(
            api_value.pointer("/mail/smtp_password/is_set"),
            Some(&serde_json::Value::Bool(true))
        );

        let mut log_value = serde_json::to_value(&config).unwrap();
        redact_config_for_log(&mut log_value);
        let rendered = serde_json::to_string(&log_value).unwrap();
        assert!(!rendered.contains("hunter2-smtp"), "log dump leaked the SMTP password");
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
}
