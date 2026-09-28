import { api } from "./api-client";
import type { ConfigUpdateResponse, SensitiveField } from "./config-types";

// Mirrors `crates/frona-server/src/inference/credential/store.rs::CredentialMethod`
// (`#[serde(rename_all = "snake_case")]`).
export type CredentialMethod = "api_key" | "oauth" | "aws" | "azure_entra" | "anonymous";

// Mirrors `crates/frona-server/src/core/config/types.rs::ApiSurface`.
export type ProviderProtocol =
  | "completions"
  | "responses"
  | "anthropic-messages"
  | "google-generate-content"
  | "amazon-bedrock-converse"
  | "cohere-chat"
  | "ollama"
  | "huggingface";

// Mirrors `AdapterId` (`#[serde(rename_all = "kebab-case")]`).
export type ProviderAdapter =
  | "openai"
  | "anthropic"
  | "gemini"
  | "bedrock"
  | "cohere"
  | "ollama"
  | "huggingface";

/** Wire shape of `ModelProviderConfig` (`core/config/types.rs`). Every field
 *  bar `enabled` is `Option<T>` server-side; most have no
 *  `skip_serializing_if`, so a saved connection echoes them back as explicit
 *  `null` rather than omitting the key - still safe to type as optional here
 *  since a present-but-`undefined`-typed field accepts both. `[key: string]`
 *  covers the flattened `attributes` bag (e.g. Azure's `azure_api_version`). */
export interface ProviderConnection {
  credential_id?: string | null;
  provider?: string | null;
  // Loosely typed (not `ProviderAdapter`) because this describes a
  // client-authored draft - `ModelProviderConfig.adapter` is a plain
  // `string | null` too, and the two need to be freely interchangeable.
  adapter?: string | null;
  enabled?: boolean;
  base_url?: string | null;
  api_key?: SensitiveField | null;
  api_version?: string | null;
  aws_region?: string | null;
  aws_profile?: string | null;
  azure_credential?: string | null;
  billing?: import("./config-types").ProviderBilling | null;
  [key: string]: unknown;
}

export interface SavedCredential {
  credential_id: string;
  integration: string;
  name: string;
}

// Mirrors `service::PublicCredential`. The server always sends `credential_id`
// (`null` rather than omitting it), but it's typed optional here so fixtures
// that don't care about it don't have to spell out the null.
export interface CredentialStatus {
  credential_id?: string | null;
  method: CredentialMethod;
  state: "active" | "pending" | "removed";
  generation: number;
  version: string;
  validation_id?: string;
}

export interface EffectiveAuthentication {
  method: CredentialMethod | null;
  source: string;
}

// Mirrors `service::AuthenticationMethodInfo`.
export interface ConfiguredAuthMethod {
  id: string;
  method: CredentialMethod;
  priority: number;
  protocols: ProviderProtocol[];
  login_available: boolean;
  validation_available: boolean;
}

// Mirrors `service::ProviderInspection`.
export interface ProviderInspection {
  setup: ProviderCatalogEntry | null;
  handle: string;
  provider: string;
  adapter: ProviderAdapter;
  configuration: ProviderConnection;
  pending_removal: boolean;
  effective_authentication: EffectiveAuthentication | null;
  authentication_methods: ConfiguredAuthMethod[];
  credentials: CredentialStatus[];
  affected_groups: string[];
}

// Mirrors `directory/providers.rs::ProviderFieldTarget` (`tag = "kind"`).
export type ProviderFieldTarget =
  | { kind: "configuration"; path: string }
  | { kind: "credential"; field: string };

export interface ProviderFieldInfo {
  id: string;
  target: ProviderFieldTarget;
  label: string;
  required: boolean;
  sensitive: boolean;
  schema: Record<string, unknown>;
  suggested_env: string[];
}

export interface AuthMethodInfo {
  id: string;
  credential_method: CredentialMethod;
  access_mode: "api" | "subscription";
  priority: number;
  protocols: ProviderProtocol[];
  interaction: "form" | "backend_login" | "ambient_credentials" | "none";
  persistence: "database" | "none";
  fields: ProviderFieldInfo[];
  validation_available: boolean;
}

export interface ProviderCatalogEntry {
  id: string;
  name: string;
  description: string | null;
  documentation_url: string | null;
  logo_url: string | null;
  adapter: ProviderAdapter;
  default_base_url: string | null;
  api_surfaces: ProviderProtocol[];
  auth_methods: AuthMethodInfo[];
  fields: ProviderFieldInfo[];
  configuration_defaults: ProviderConnection;
  sources: string[];
  warnings: string[];
  catalog_available: boolean;
  effective_protocols: ProviderProtocol[] | null;
}

export interface CatalogSourceStatus {
  source: string;
  origin: "bundled" | "cache" | "remote" | null;
  state: "fresh" | "stale" | "unavailable";
  version: string | null;
  fetched_at: string | null;
  last_error: string | null;
}

export interface ProviderCatalog {
  providers: ProviderCatalogEntry[];
  source_status: Record<string, CatalogSourceStatus>;
}

export function getProviderCatalog(): Promise<ProviderCatalog> {
  return api.get<ProviderCatalog>("/api/config/provider-catalog");
}

// Mirrors `service::CredentialInput` (`tag = "source"`).
export type CredentialInput =
  | { source: "api_key"; api_key: string }
  | { source: "database"; method: CredentialMethod }
  | { source: "environment"; variable: string }
  | { source: "ambient"; method: CredentialMethod }
  | { source: "anonymous" };

export type SettingApplicability =
  | { op: "all"; expressions: SettingApplicability[] }
  | { op: "any"; expressions: SettingApplicability[] }
  | { op: "not"; expression: SettingApplicability }
  | { op: "in"; config_path: string; values: unknown[] };

export interface ModelSettingInfo {
  id: string;
  storage: { kind: "typed" | "extra_params"; config_path: string } | null;
  request_path: string | null;
  catalog_path: string | null;
  label: string;
  description: string | null;
  group: string;
  scope: "provider_request" | "frona_runtime";
  schema: Record<string, unknown>;
  applicability: SettingApplicability | null;
  support: "typed" | "extra_params" | "unsupported_by_adapter" | "configured_unverified";
  sources: string[];
}

export interface ModelProtocolInfo {
  api: ProviderProtocol;
  available: boolean;
  settings: ModelSettingInfo[];
  warnings: string[];
}

export interface ModelCapabilities {
  reasoning: boolean | null;
  tool_call: boolean | null;
  structured_output: boolean | null;
  input: string[];
  output: string[];
}

// Mirrors `directory/models.rs::ProviderModelInfo`.
export interface ProviderModelRow {
  id: string;
  name: string | null;
  description: string | null;
  context_window: number | null;
  max_tokens: number | null;
  suggested_protocol: ProviderProtocol | null;
  protocols: ModelProtocolInfo[];
  availability: "account" | "catalog" | "unverified" | "configured" | "recipe";
  configured_in: string[];
  capabilities: ModelCapabilities;
  sources: string[];
  warnings: string[];
}

// Mirrors `service::ModelListing`.
export interface ModelDirectory {
  connection: string;
  credential_method: CredentialMethod | null;
  access_mode: "api" | "subscription";
  source: string;
  directory_status: "live" | "catalog_fallback" | "configured_only" | "live_error" | "unavailable";
  source_status: Record<string, unknown>;
  manual_entry: boolean;
  models: ProviderModelRow[];
}
export type ProviderModelListing = ModelDirectory;

// Mirrors `service::ValidationResult`.
export interface ProviderValidation {
  validation_id: string;
  credential: CredentialStatus;
  models: ProviderModelRow[] | null;
}

// Mirrors `service::DraftRequest`.
export interface DraftProvider {
  manual_models?: string[];
  config: ProviderConnection;
  validation_id: string;
  method: CredentialMethod;
  source: string;
}

// Mirrors `service::MutationResult`.
export interface CredentialMutation {
  credential: CredentialStatus;
  affected_groups: string[];
  unavailable_models: Record<string, [string, string][]>;
}

// Mirrors `credential/setup.rs::LoginAttempt` and
// `credential/managed/login/provider.rs::LoginChallenge` (`tag = "kind"`).
export interface LoginAttempt {
  id: string;
  status: "pending" | "validated" | "failed" | "expired" | "cancelled";
  // Not a discriminated union on `kind`: the server's `Redirect` variant never
  // carries `user_code`, but the UI only branches on whether the field is
  // present, so a flat optional shape is what the rendering code (and its
  // tests) actually rely on.
  challenge: { kind: "redirect" | "device_code"; url: string; user_code?: string; message?: string | null } | null;
  credential_id: string | null;
  error?: string;
}

const path = (handle: string) => `/api/config/providers/${encodeURIComponent(handle.trim().toLowerCase())}`;

export const providerAdmin = {
  list: () => api.get<{ providers: ProviderInspection[] }>("/api/config/providers"),
  savedCredentials: () => api.get<SavedCredential[]>("/api/config/provider-credentials"),
  environmentVariables: () => api.get<string[]>("/api/config/environment-variables"),
  accept: (handle: string, draft: DraftProvider) =>
    api.post<CredentialStatus>(`${path(handle)}/credentials`, {
      config: draft.config,
      validation_id: draft.validation_id,
      method: draft.method,
      source: draft.source,
    }),
  inspect: (handle: string) => api.get<ProviderInspection>(path(handle)),
  inspectDraft: (handle: string, config: ProviderConnection) =>
    api.post<ProviderInspection>(`${path(handle)}/inspect`, { config }),
  validate: (handle: string, config: ProviderConnection, credential: CredentialInput) =>
    api.post<ProviderValidation>(`${path(handle)}/validate`, { config, credential }),
  models: (handle: string, manualModels: string[] = []) => {
    const query = new URLSearchParams();
    for (const model of manualModels) query.append("manual_model", model);
    return api.get<ProviderModelListing>(`${path(handle)}/models${query.size ? `?${query}` : ""}`);
  },
  draftModels: (handle: string, draft: DraftProvider) =>
    api.post<ProviderModelListing>(`${path(handle)}/models`, {
      config: draft.config,
      validation_id: draft.validation_id,
      method: draft.method,
      source: draft.source,
      ...(draft.manual_models ? { manual_models: draft.manual_models } : {}),
    }),
  credentialModels: (handle: string, config: ProviderConnection, manualModels: string[] = []) =>
    api.post<ProviderModelListing>(`${path(handle)}/models`, {
      // Config responses mask API keys as objects; those markers aren't credentials.
      config: config.api_key && typeof config.api_key === "object" ? { ...config, api_key: null } : config,
      manual_models: manualModels,
    }),
  edit: (handle: string, config: ProviderConnection, revision: string) =>
    api.put<ConfigUpdateResponse>(path(handle), { config, expected_persisted_revision: revision }),
  delete: (handle: string, revision: string) =>
    api.delete<ConfigUpdateResponse>(path(handle), { expected_persisted_revision: revision }),
  logout: (handle: string, method: CredentialMethod, generation: number) =>
    api.delete<CredentialMutation>(`${path(handle)}/credentials/${method}`, { expected_generation: generation }),
  discard: (handle: string, validationId: string) =>
    api.delete<{ discarded: boolean }>(`${path(handle)}/drafts/${encodeURIComponent(validationId)}`),
  startLogin: (handle: string, config: ProviderConnection, method: CredentialMethod) =>
    api.post<LoginAttempt>(`${path(handle)}/login/start`, { config, method }),
  loginStatus: (handle: string, attempt: string) =>
    api.get<LoginAttempt>(`${path(handle)}/login/${encodeURIComponent(attempt)}`),
  completeLogin: (handle: string, attempt: string, code: string) =>
    api.post<LoginAttempt>(`${path(handle)}/login/${encodeURIComponent(attempt)}/complete`, { code }),
  cancelLogin: (handle: string, attempt: string) =>
    api.delete<LoginAttempt>(`${path(handle)}/login/${encodeURIComponent(attempt)}`),
};
