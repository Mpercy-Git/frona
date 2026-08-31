"use client";

import { useState, useEffect, useRef } from "react";
import type { ModelProviderConfig, SensitiveField } from "@/lib/config-types";
import { getProviderModels } from "@/lib/config-types";
import { SensitiveInput, TextInput, SectionHeader } from "@/components/settings/field";
import { CloudIcon } from "@heroicons/react/24/outline";

const KNOWN_PROVIDERS = [
  "anthropic",
  "openai",
  "groq",
  "openrouter",
  "deepseek",
  "gemini",
  "cohere",
  "mistral",
  "perplexity",
  "together",
  "xai",
  "hyperbolic",
  "moonshot",
  "mira",
  "galadriel",
  "huggingface",
<<<<<<< HEAD
  "azure",
=======
  "zai",
  "venice",
  "minimax",
>>>>>>> origin/main
  "ollama",
  "llamafile",
  "generic",
];

export function formatProviderName(id: string): string {
  const names: Record<string, string> = {
    anthropic: "Anthropic",
    openai: "OpenAI",
    groq: "Groq",
    openrouter: "OpenRouter",
    deepseek: "DeepSeek",
    gemini: "Gemini",
    cohere: "Cohere",
    mistral: "Mistral",
    perplexity: "Perplexity",
    together: "Together",
    xai: "xAI",
    hyperbolic: "Hyperbolic",
    moonshot: "Moonshot",
    mira: "Mira",
    galadriel: "Galadriel",
    huggingface: "Hugging Face",
<<<<<<< HEAD
    azure: "Azure OpenAI",
=======
    zai: "Z.ai",
    venice: "Venice",
    minimax: "MiniMax",
>>>>>>> origin/main
    ollama: "Ollama",
    llamafile: "llamafile",
    generic: "OpenAI-compatible",
  };
  return names[id] ?? id;
}

export type TestStatus = "idle" | "testing" | "success" | "error";

export interface ProviderState {
  id: string;
  api_key: SensitiveField;
  base_url: string | null;
  api_version: string | null;
  enabled: boolean;
  testStatus: TestStatus;
  /** Provider error message when testStatus === "error". */
  testError?: string;
}

function hasKey(p: ProviderState): boolean {
  if (p.id === "ollama") return true;
  if (typeof p.api_key === "string") return p.api_key.length > 0;
  if (typeof p.api_key === "object" && p.api_key?.is_set) return true;
  return false;
}

/** Build provider states from config, preserving existing test statuses */
function buildStates(
  providers: Record<string, ModelProviderConfig>,
  prev: ProviderState[]
): ProviderState[] {
  const prevMap = new Map(prev.map((p) => [p.id, p]));
  return Object.entries(providers).map(([id, cfg]) => {
    const existing = prevMap.get(id);
    return {
      id,
      api_key: cfg.api_key,
      base_url: cfg.base_url,
      api_version: cfg.api_version ?? null,
      enabled: cfg.enabled !== false,
      testStatus: existing?.testStatus ?? "idle" as TestStatus,
      testError: existing?.testError,
    };
  });
}

/** Compute block reason from provider states. null = ready to proceed. */
export function computeBlockReason(states: ProviderState[]): string | null {
  const enabled = states.filter((p) => p.enabled);
  if (enabled.length === 0) return "Enable at least one provider to continue";

  const noKey = enabled.filter((p) => !hasKey(p));
  if (noKey.length > 0) {
    const names = noKey.map((p) => formatProviderName(p.id)).join(", ");
    return `${names} — missing API key`;
  }

  const failing = enabled.filter((p) => p.testStatus === "error");
  if (failing.length > 0) {
    const names = failing.map((p) => formatProviderName(p.id)).join(", ");
    return `${names} — failed verification, fix or remove to continue`;
  }

  const pending = enabled.filter((p) => p.testStatus === "testing" || p.testStatus === "idle");
  if (pending.length > 0) return "Verifying providers...";

  return null;
}

/** Test all enabled providers that have keys and aren't already verified */
async function testAllProviders(
  states: ProviderState[],
  onUpdate: (id: string, status: TestStatus, error?: string) => void
): Promise<void> {
  const toTest = states.filter((p) => p.enabled && hasKey(p) && p.testStatus === "idle");
  await Promise.all(
    toTest.map(async (p) => {
      onUpdate(p.id, "testing");
      try {
        const key = typeof p.api_key === "string" ? p.api_key : undefined;
        await getProviderModels(p.id, {
          apiKey: key || undefined,
          baseUrl: p.base_url ?? undefined,
        });
        onUpdate(p.id, "success");
      } catch (e) {
        onUpdate(p.id, "error", cleanProviderError(e));
      }
    })
  );
}

/** Extract a concise, user-facing message from a provider test failure. */
function cleanProviderError(e: unknown): string {
  const raw = e instanceof Error ? e.message : "Test failed";
  // Surface the upstream provider's own error text when present. Try to pull a
  // JSON `message` field out of the wrapped body; fall back to the raw string.
  const jsonStart = raw.indexOf("{");
  if (jsonStart >= 0) {
    try {
      const parsed = JSON.parse(raw.slice(jsonStart));
      const msg = parsed?.error?.message ?? parsed?.message;
      const reason = parsed?.error?.details?.[0]?.reason;
      if (typeof msg === "string") {
        return reason ? `${msg} (${reason})` : msg;
      }
    } catch {
      // not JSON — fall through
    }
  }
  return raw;
}

export function TestStatusIcon({ status }: { status: TestStatus }) {
  if (status === "testing") {
    return (
      <svg className="h-4 w-4 animate-spin text-text-tertiary" viewBox="0 0 24 24" fill="none">
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
    );
  }
  if (status === "success") {
    return (
      <svg className="h-4 w-4 text-green-500" viewBox="0 0 20 20" fill="currentColor">
        <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
      </svg>
    );
  }
  if (status === "error") {
    return (
      <svg className="h-4 w-4 text-red-500" viewBox="0 0 20 20" fill="currentColor">
        <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z" clipRule="evenodd" />
      </svg>
    );
  }
  return null;
}

interface ProviderCardProps {
  state: ProviderState;
  onChange: (updated: ModelProviderConfig) => void;
  onToggle: (enabled: boolean) => void;
}

function ProviderCard({ state, onChange, onToggle }: ProviderCardProps) {
  const isAzure = state.id === "azure";
  // Spread the current values rather than rebuilding the object field by
  // field, so adding a field to ModelProviderConfig can't silently drop it on
  // the next edit of a different field.
  const patch = (fields: Partial<ModelProviderConfig>) =>
    onChange({
      api_key: state.api_key,
      base_url: state.base_url,
      api_version: state.api_version,
      enabled: state.enabled,
      ...fields,
    });
  return (
    <div className="rounded-lg border border-border bg-surface-secondary p-4 space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <h4 className="text-sm font-medium text-text-primary">
            {formatProviderName(state.id)}
          </h4>
          <TestStatusIcon status={state.testStatus} />
        </div>
        <button
          type="button"
          onClick={() => onToggle(!state.enabled)}
          title={state.enabled ? "Disable provider (keeps API key)" : "Enable provider"}
          className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${
            state.enabled ? "bg-accent" : "bg-surface-tertiary"
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-surface shadow transform transition-transform ${
              state.enabled ? "translate-x-5" : "translate-x-0"
            }`}
          />
        </button>
      </div>

      <SensitiveInput
        label="API Key"
        value={state.api_key}
        onChange={(value) => patch({ api_key: value })}
        placeholder="Enter API key"
      />

      <TextInput
        label="Base URL"
        value={state.base_url}
        onChange={(value) => patch({ base_url: value || null })}
        placeholder={isAzure ? "https://<resource>.openai.azure.com" : "Optional custom base URL"}
      />

      {isAzure && (
        <TextInput
          label="API Version"
          description="Azure versions its data plane in the query string, and the version gates which request fields are accepted. Leave blank for the client default."
          value={state.api_version}
          onChange={(value) => patch({ api_version: value || null })}
          placeholder="2024-10-21"
        />
      )}

      {state.testStatus === "error" && state.testError && (
        <p className="rounded-lg bg-error-bg px-3 py-2 text-xs text-error-text break-words">
          {state.testError}
        </p>
      )}
    </div>
  );
}

function CollapsedProvider({ id, onEnable }: { id: string; onEnable: () => void }) {
  return (
    <div className="flex items-center justify-between rounded-lg border border-border bg-surface-secondary px-4 py-3">
      <span className="text-sm text-text-secondary">
        {formatProviderName(id)}
      </span>
      <button
        type="button"
        onClick={onEnable}
        className="rounded-lg bg-surface-tertiary px-3 py-1 text-xs font-medium text-text-secondary hover:bg-accent hover:text-surface transition"
      >
        Enable
      </button>
    </div>
  );
}

interface ProvidersSectionProps {
  providers: Record<string, ModelProviderConfig>;
  onChange: (providers: Record<string, ModelProviderConfig>) => void;
  onReadyChange?: (blockReason: string | null) => void;
}

export function ProvidersSection({ providers, onChange, onReadyChange }: ProvidersSectionProps) {
  const [providerStates, setProviderStates] = useState<ProviderState[]>(() =>
    buildStates(providers, [])
  );
  const testingRef = useRef(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Sync provider states when providers prop changes (preserve test statuses)
  const [prevProviders, setPrevProviders] = useState(providers);
  if (providers !== prevProviders) {
    setPrevProviders(providers);
    setProviderStates((prev) => buildStates(providers, prev));
  }

  // Notify parent of readiness whenever states change
  useEffect(() => {
    onReadyChange?.(computeBlockReason(providerStates));
  }, [providerStates, onReadyChange]);

  // Debounced auto-test: whenever states change, schedule a test run
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);

    const needsTest = providerStates.some(
      (p) => p.enabled && hasKey(p) && p.testStatus === "idle"
    );
    if (!needsTest || testingRef.current) return;

    debounceRef.current = setTimeout(() => {
      testingRef.current = true;
      testAllProviders(providerStates, (id, status, error) => {
        setProviderStates((prev) =>
          prev.map((p) =>
            p.id === id
              ? { ...p, testStatus: status, testError: status === "error" ? error : undefined }
              : p,
          ),
        );
      }).finally(() => {
        testingRef.current = false;
      });
    }, 800);

    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
   
  }, [providerStates]);

  const enabledIds = Object.entries(providers)
    .filter(([, config]) => config.enabled !== false)
    .map(([id]) => id);
  const sortedEnabled = [
    ...KNOWN_PROVIDERS.filter((id) => enabledIds.includes(id)),
    ...enabledIds.filter((id) => !KNOWN_PROVIDERS.includes(id)),
  ];
  const disabledIds = [
    ...KNOWN_PROVIDERS.filter((id) => !(id in providers) || providers[id].enabled === false),
    ...Object.entries(providers)
      .filter(([id, config]) => config.enabled === false && !KNOWN_PROVIDERS.includes(id))
      .map(([id]) => id),
  ];

  const updateProvider = (id: string, updated: ModelProviderConfig) => {
    setProviderStates((prev) =>
      prev.map((p) => (p.id === id ? { ...p, testStatus: "idle" as TestStatus } : p))
    );
    onChange({ ...providers, [id]: updated });
  };

  const enableProvider = (id: string) => {
    onChange({
      ...providers,
      [id]: providers[id]
        ? { ...providers[id], enabled: true }
        : { api_key: "", base_url: null, enabled: true },
    });
  };

  // Toggle a provider on/off while KEEPING its api_key/base_url, so a disabled
  // provider can be re-enabled without re-entering the key. We send enabled:
  // false (not a deletion) — enabled defaults to true, so the false value is
  // persisted by strip_defaults and survives the deep_merge round-trip.
  const setProviderEnabled = (id: string, enabled: boolean) => {
    const current = providers[id];
    if (!current) return;
    updateProvider(id, {
      api_key: current.api_key,
      base_url: current.base_url,
      api_version: current.api_version,
      enabled,
    });
  };

  return (
    <div className="space-y-4">
      <SectionHeader title="Providers" description="Configure your LLM API providers" icon={CloudIcon} />
      {sortedEnabled.length > 0 && (
        <div className="space-y-3">
          {sortedEnabled.map((id) => {
            const state = providerStates.find((p) => p.id === id);
            if (!state) return null;
            return (
              <ProviderCard
                key={id}
                state={state}
                onChange={(updated) => updateProvider(id, updated)}
                onToggle={(enabled) => setProviderEnabled(id, enabled)}
              />
            );
          })}
        </div>
      )}

      {disabledIds.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium text-text-tertiary pt-2">Available Providers</h3>
          {disabledIds.map((id) => (
            <CollapsedProvider
              key={id}
              id={id}
              onEnable={() => enableProvider(id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
