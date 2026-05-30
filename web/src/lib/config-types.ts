import { api } from "./api-client";

// Mirrors Rust Config struct — sensitive fields come as { is_set: boolean } from GET
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
  callback_base_url: string | null;
  inbound_enabled: boolean;
  inbound_user_id: string | null;
  inbound_agent_id: string | null;
  inbound_welcome_greeting: string | null;
  inbound_allowlist: string[];
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
  keeper_app_key: SensitiveField;
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

export interface ModelGroupConfig {
  provider: string;
  model: string;
  fallbacks?: ModelGroupConfig[];
  max_tokens?: number | null;
  temperature?: number | null;
  context_window?: number | null;
  retry?: RetryConfig;
  // Anthropic
  thinking?: AnthropicThinking | null;
  top_p?: number | null;
  top_k?: number | null;
  stop_sequences?: string[] | null;
  // Ollama
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
  // OpenAI-compatible
  max_completion_tokens?: number | null;
  reasoning_effort?: string | null;
  logprobs?: boolean | null;
  top_logprobs?: number | null;
  // Gemini
  thinking_config?: GeminiThinkingConfig | null;
  candidate_count?: number | null;
  // Generic catch-all
  [key: string]: unknown;
}

export interface ModelProviderConfig {
  api_key: SensitiveField;
  base_url: string | null;
  enabled: boolean;
}

export interface InferenceConfig {
  max_tool_turns: number;
  default_max_tokens: number;
  compaction_trigger_pct: number;
  history_truncation_pct: number;
}

export interface SchedulerConfig {
  space_compaction_secs: number;
  memory_compaction_secs: number;
  poll_secs: number;
}

export interface AppConfig {
  port_range_start: number;
  port_range_end: number;
  health_check_timeout_secs: number;
  max_restart_attempts: number;
  hibernate_after_secs: number;
}

export interface Config {
  server: ServerConfig;
  sandbox: SandboxConfig;
  auth: AuthConfig;
  sso: SsoConfig;
  browser: BrowserConfig | null;
  search: SearchConfig;
  voice: VoiceConfig;
  vault: VaultConfig;
  inference: InferenceConfig;
  scheduler: SchedulerConfig;
  app: AppConfig;
  models: Record<string, ModelGroupConfig>;
  providers: Record<string, ModelProviderConfig>;
}

// JSON Schema types (subset we need for rendering)
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
  $ref?: string;
}

export interface ConfigUpdateResponse {
  config: Config;
  restart_required: boolean;
}

export function getConfigSchema(): Promise<JsonSchema> {
  return api.get<JsonSchema>("/api/config/schema");
}

export function getConfig(): Promise<Config> {
  return api.get<Config>("/api/config");
}

function stripRedactedSensitiveFields(obj: unknown): unknown {
  if (obj === null || obj === undefined) return obj;
  if (typeof obj !== "object") return obj;
  if (Array.isArray(obj)) return obj.map(stripRedactedSensitiveFields);
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
    if (typeof value === "object" && value !== null && "is_set" in value) continue;
    result[key] = stripRedactedSensitiveFields(value);
  }
  return result;
}

export function updateConfig(patch: Record<string, unknown>): Promise<ConfigUpdateResponse> {
  return api.put<ConfigUpdateResponse>("/api/config", stripRedactedSensitiveFields(patch) as Record<string, unknown>);
}

export interface ModelInfo {
  id: string;
  name?: string;
  context_window?: number;
  max_tokens?: number;
}

export function getProviderModels(
  providerId: string,
  opts?: { apiKey?: string; baseUrl?: string }
): Promise<{ models: ModelInfo[] }> {
  const params = new URLSearchParams();
  if (opts?.apiKey) params.set("api_key", opts.apiKey);
  if (opts?.baseUrl) params.set("base_url", opts.baseUrl);
  const qs = params.toString();
  return api.get<{ models: ModelInfo[] }>(
    `/api/config/providers/${providerId}/models${qs ? `?${qs}` : ""}`
  );
}

export function isSensitiveSet(value: SensitiveField): boolean {
  if (typeof value === "object" && value !== null && "is_set" in value) {
    return value.is_set;
  }
  return typeof value === "string" && value.length > 0;
}
