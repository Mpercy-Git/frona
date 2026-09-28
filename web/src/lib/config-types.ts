import { api } from "./api-client";

// Mirrors Rust Config struct - sensitive fields come as { is_set: boolean } from GET
export type SensitiveField = string | { is_set: boolean };

export interface ServerConfig {
  port: number;
  static_dir: string;
  issuer_url: string;
  max_concurrent_tasks: number;
  cors_origins: string | null;
  base_url: string | null;
  backend_url: string | null;
  frontend_url: string | null;
  external_url: string | null;
  max_body_size_bytes: number;
  /** Default IANA timezone for users with no profile timezone set.
      Empty string → auto-detect from TZ env var / /etc/localtime / fall back to UTC. */
  timezone: string;
}

export interface SandboxConfig {
  disabled: boolean;
  /** Flattened from `default_limits: SandboxLimits` server-side, so the
      JSON shape stays one level deep. */
  max_cpu_pct: number;
  max_memory_pct: number;
  timeout_secs: number;
  max_total_cpu_pct: number;
  max_total_memory_pct: number;
  default_network_access: boolean;
}

export interface AuthConfig {
  encryption_secret: SensitiveField;
  access_token_expiry_secs: number;
  refresh_token_expiry_secs: number;
  presign_expiry_secs: number;
  allow_registration: boolean;
  max_login_attempts: number;
  lockout_minutes: number;
  password_reset_expiry_minutes: number;
}

export type SmtpTls = "starttls" | "implicit" | "none";

export interface MailConfig {
  smtp_host: string;
  smtp_port: number;
  smtp_username: string | null;
  smtp_password: SensitiveField;
  tls: SmtpTls;
  from_address: string;
  from_name: string;
}

export interface SsoConfig {
  enabled: boolean;
  authority: string | null;
  client_id: string | null;
  client_secret: SensitiveField;
  scopes: string;
  allow_unknown_email_verification: boolean;
  client_cache_expiration: number;
  disable_local_auth: boolean;
  signups_match_email: boolean;
}

export interface BrowserConfig {
  ws_url: string;
  profiles_path: string;
  connection_timeout_ms: number;
}

export interface SearchConfig {
  provider: string | null;
  searxng_base_url: string | null;
}

export interface VoiceConfig {
  provider: string | null;
  twilio_account_sid: SensitiveField;
  twilio_auth_token: SensitiveField;
  twilio_from_number: string | null;
  twilio_voice_id: string | null;
  twilio_speech_model: string | null;
  twilio_tts_provider: string | null;
  twilio_elevenlabs_text_normalization: string | null;
  twilio_language: string | null;
  twilio_interrupt_sensitivity: string | null;
  callback_base_url: string | null;
  inbound_enabled: boolean;
  silence_fill_enabled: boolean;
  silence_fill_initial_delay_secs: number;
  silence_fill_interval_secs: number;
  silence_fill_phrases: string[];
}

export interface VaultConfig {
  onepassword_service_account_token: SensitiveField;
  onepassword_vault_id: string | null;
  bitwarden_client_id: string | null;
  bitwarden_client_secret: SensitiveField;
  bitwarden_master_password: SensitiveField;
  bitwarden_server_url: string | null;
  hashicorp_address: string | null;
  hashicorp_token: SensitiveField;
  hashicorp_mount: string | null;
  keepass_path: string | null;
  keepass_password: SensitiveField;
}

export interface RetryConfig {
  max_retries: number;
  initial_backoff_ms: number;
  backoff_multiplier: number;
  max_backoff_ms: number;
}

export interface AnthropicThinking {
  type: string;
  budget_tokens?: number | null;
}

export interface GeminiThinkingConfig {
  thinking_budget: number;
  include_thoughts?: boolean | null;
}

export interface OpenRouterMaxPrice {
  prompt?: number | null;
  completion?: number | null;
  request?: number | null;
  image?: number | null;
}

/** Maps to the `provider` object in the OpenRouter API request. */
export interface OpenRouterProviderRouting {
  order?: string[] | null;
  only?: string[] | null;
  ignore?: string[] | null;
  allow_fallbacks?: boolean | null;
  require_parameters?: boolean | null;
  quantizations?: string[] | null;
  sort?: string | null;
  max_price?: OpenRouterMaxPrice | null;
  data_collection?: string | null;
  zdr?: boolean | null;
}

export interface ModelGroupConfig {
  provider: string;
  model: string;
  /** Protocol/API surface override. See `crates/frona-server/src/core/config/types.rs::ApiSurface`. */
  api?: import("./provider-admin").ProviderProtocol;
  /** Arbitrary request fields with no typed home on `ModelGroupConfig` yet -
   *  passed through to the provider's request body verbatim. */
  extra_params?: Record<string, unknown>;
  fallbacks?: ModelGroupConfig[];
  max_tokens?: number | null;
  temperature?: number | null;
  context_window?: number | null;
  retry?: RetryConfig;
  thinking?: AnthropicThinking | null;
  top_p?: number | null;
  top_k?: number | null;
  stop_sequences?: string[] | null;
  think?: boolean | null;
  num_ctx?: number | null;
  num_predict?: number | null;
  num_batch?: number | null;
  num_keep?: number | null;
  num_thread?: number | null;
  num_gpu?: number | null;
  min_p?: number | null;
  repeat_penalty?: number | null;
  repeat_last_n?: number | null;
  frequency_penalty?: number | null;
  presence_penalty?: number | null;
  mirostat?: number | null;
  mirostat_eta?: number | null;
  mirostat_tau?: number | null;
  tfs_z?: number | null;
  seed?: number | null;
  stop?: string[] | null;
  use_mmap?: boolean | null;
  use_mlock?: boolean | null;
  max_completion_tokens?: number | null;
  reasoning_effort?: string | null;
  logprobs?: boolean | null;
  top_logprobs?: number | null;
  thinking_config?: GeminiThinkingConfig | null;
  candidate_count?: number | null;
  /** OpenRouter: the only value the API accepts is "fallback". */
  route?: string | null;
  provider_routing?: OpenRouterProviderRouting | null;
  /** OpenRouter: place a cache_control breakpoint on the system prompt. Defaults to true. */
  prompt_caching?: boolean | null;
  [key: string]: unknown;
}

export interface ModelProviderConfig {
  /** Stable ID of an authorized managed credential (`credential/managed`).
   *  Set once a connection's credential is accepted; mutually exclusive in
   *  practice with a literal `api_key`. */
  credential_id?: string | null;
  /** Provider brand (e.g. "openai", "byteplus"). Legacy entries infer it from
   *  the providers-map key instead. */
  provider?: string | null;
  /** Compiled logical adapter for dynamic/direct-YAML brands. See
   *  `crates/frona-server/src/core/config/types.rs::AdapterId`. */
  adapter?: string | null;
  api_key: SensitiveField | null;
  base_url: string | null;
  /** Azure OpenAI only — its data plane is versioned in the query string. */
  api_version?: string | null;
  enabled: boolean;
  /** How this provider charges. Affects cost reporting only, never routing.
   *  Absent means unstated, which the server resolves per provider: local
   *  runtimes as self-hosted, everything else as pay-as-you-go. */
  billing?: ProviderBilling | null;
  /** Other adapter-specific fields (`aws_region`, `aws_profile`,
   *  `azure_credential`, and the flattened `attributes` bag - e.g. Azure's
   *  `azure_api_version`) round-trip through here rather than being
   *  enumerated one by one. */
  [key: string]: unknown;
}

export type ProviderBillingKind = "metered" | "subscription" | "self_hosted";

export interface ProviderBilling {
  kind: ProviderBillingKind;
  monthly_cost?: number | null;
  currency?: string | null;
  included_tokens?: number | null;
  included_spend_usd?: number | null;
  overage_is_metered: boolean;
  renewal_day?: number | null;
  notes?: string | null;
}

export interface InferenceConfig {
  max_tool_turns: number;
  default_max_tokens: number;
  compaction_trigger_pct: number;
  history_truncation_pct: number;
  tool_timeout_secs: number;
  vision_models: string[];
  text_only_models: string[];
  transcribe_when_vision_unknown: boolean;
}

export interface SchedulerConfig {
  poll_secs: number;
}

export interface AppConfig {
  port_range_start: number;
  port_range_end: number;
  health_check_timeout_secs: number;
  max_restart_attempts: number;
  hibernate_after_secs: number;
}

export type MemoryBackend = "basic" | "pkm";

export interface MemoryConfig {
  /** `null` = unconfigured; the server resolves it at boot (PKM for a fresh install,
   *  Basic for an existing one). The UI renders `null` as a concrete selection. */
  backend: MemoryBackend | null;
  model_group: string;
  basic_compaction_token_threshold: number;
  basic_compaction_secs: number;
  basic_space_compaction_secs: number;
  pkm_search_top_k: number;
  pkm_max_lookups_per_turn: number;
  pkm_short_memory_half_life_secs: number;
  pkm_short_memory_demote_threshold: number;
  pkm_short_memory_top_n: number;
  pkm_short_memory_token_cap: number;
  pkm_playbook_index_token_cap: number;
  pkm_consolidate_secs: number;
  pkm_consolidate_idle_secs: number;
  pkm_consolidation_concurrency: number;
  pkm_consolidation_max_tool_turns: number;
  pkm_consolidation_max_submissions: number;
  pkm_playbook_max_tool_turns: number;
  pkm_playbook_max_submissions: number;
  pkm_extract_max_tokens: number;
  pkm_extract_max_messages: number;
  pkm_extract_agent_evidence_lookback_messages: number;
  pkm_extract_agent_evidence_result_token_cap: number;
  pkm_consolidation_max_attempts: number;
  pkm_adjudication_max_attempts_per_batch: number;
  pkm_consolidation_checkpoint_failure_cap: number;
  pkm_consolidation_retry_base_secs: number;
}

export interface Config {
  server: ServerConfig;
  sandbox: SandboxConfig;
  auth: AuthConfig;
  sso: SsoConfig;
  mail: MailConfig;
  browser: BrowserConfig | null;
  search: SearchConfig;
  voice: VoiceConfig;
  vault: VaultConfig;
  inference: InferenceConfig;
  scheduler: SchedulerConfig;
  app: AppConfig;
  memory: MemoryConfig;
  models: Record<string, ModelGroupConfig>;
  providers: Record<string, ModelProviderConfig>;
}

export interface JsonSchemaProperty {
  type?: string;
  description?: string;
  default?: unknown;
  enum?: string[];
  "x-sensitive"?: boolean;
  properties?: Record<string, JsonSchemaProperty>;
  $ref?: string;
}

export interface JsonSchema {
  properties?: Record<string, JsonSchemaProperty>;
  definitions?: Record<string, JsonSchemaProperty>;
  // schemars 1.x (this fork's version) emits draft 2020-12 `$defs` rather than
  // the older `definitions` keyword; both are read so a schema built either
  // way resolves.
  $defs?: Record<string, JsonSchemaProperty>;
  $ref?: string;
}

/** The shape `super::config::response()` (`api/routes/config.rs`) produces for
 *  both `PUT /api/config` and every provider-admin mutation route, and that
 *  `GET /api/config` now also returns. Unlike upstream, this fork's backend
 *  has no `authoring_document`/`parameter_overrides` fields — it doesn't
 *  track per-field parameter-override provenance the way upstream's
 *  post-Group-C config service does. */
export interface ConfigUpdateResponse {
  config: Config;
  persisted_revision: string;
  active_revision: string;
  restart_required: boolean;
}

export function getConfigSchema(): Promise<JsonSchema> {
  return api.get<JsonSchema>("/api/config/schema");
}

export function getConfigDocument(): Promise<ConfigUpdateResponse> {
  return api.get<ConfigUpdateResponse>("/api/config");
}

export async function getConfig(): Promise<Config> {
  const document = await getConfigDocument();
  return document.config;
}

/** Provider-field sensitivity plus the handful of other single-field secrets
 *  redacted by `redact_config_for_api` (`core/config/document.rs::SENSITIVE_PATHS`
 *  / `SENSITIVE_PROVIDER_FIELDS`). Path-scoped rather than "any `{is_set}`
 *  object anywhere" so a `models.*.extra_params` value that happens to look
 *  like `{is_set: true}` (arbitrary passthrough request JSON) isn't mistaken
 *  for a redaction marker and silently dropped from the patch.
 */
const SENSITIVE_FIELD_PATHS: [string, string][] = [
  ["auth", "encryption_secret"],
  ["sso", "client_secret"],
  ["voice", "twilio_account_sid"],
  ["voice", "twilio_auth_token"],
  ["vault", "onepassword_service_account_token"],
  ["vault", "bitwarden_client_secret"],
  ["vault", "bitwarden_master_password"],
  ["vault", "hashicorp_token"],
  ["vault", "keepass_password"],
  ["mail", "smtp_password"],
  ["push", "vapid_private_key"],
];

function stripRedactedSensitiveFields(obj: unknown, path: string[] = []): unknown {
  if (obj === null || obj === undefined) return obj;
  if (typeof obj !== "object") return obj;
  if (Array.isArray(obj)) {
    return obj.map((value, index) => stripRedactedSensitiveFields(value, [...path, String(index)]));
  }
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
    const nextPath = [...path, key];
    const sensitive =
      (nextPath.length === 3 && nextPath[0] === "providers" && key === "api_key") ||
      (nextPath.length === 2 &&
        SENSITIVE_FIELD_PATHS.some(([section, field]) => nextPath[0] === section && key === field));
    if (
      sensitive &&
      typeof value === "object" &&
      value !== null &&
      "is_set" in value &&
      Object.keys(value).length === 1 &&
      typeof (value as { is_set: unknown }).is_set === "boolean"
    ) {
      continue;
    }
    result[key] = stripRedactedSensitiveFields(value, nextPath);
  }
  return result;
}

export function updateConfig(
  patch: Record<string, unknown>,
  metadata?: { expectedPersistedRevision?: string },
): Promise<ConfigUpdateResponse> {
  const cleaned = stripRedactedSensitiveFields(patch) as Record<string, unknown>;
  // `PUT /api/config` (`api/routes/config.rs::update_config`) always expects
  // `{patch, expected_persisted_revision}` - `expected_persisted_revision` is
  // optional server-side (skips the optimistic-concurrency check when
  // absent), but the `patch` wrapper itself is not.
  return api.put<ConfigUpdateResponse>("/api/config", {
    patch: cleaned,
    expected_persisted_revision: metadata?.expectedPersistedRevision,
  });
}

export function isSensitiveSet(value: SensitiveField): boolean {
  if (typeof value === "object" && value !== null && "is_set" in value) {
    return value.is_set;
  }
  return typeof value === "string" && value.length > 0;
}
