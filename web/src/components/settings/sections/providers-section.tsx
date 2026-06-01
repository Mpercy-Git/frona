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
  "ollama",
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
    ollama: "Ollama",
  };
  return names[id] ?? id;
}

export type TestStatus = "idle" | "testing" | "success" | "error";

export interface ProviderState {
  id: string;
  api_key: SensitiveField;
  base_url: string | null;
  enabled: boolean;
  testStatus: TestStatus;
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
      enabled: cfg.enabled,
      testStatus: existing?.testStatus ?? "idle" as TestStatus,
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
  onUpdate: (id: string, status: TestStatus) => void
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
      } catch {
        onUpdate(p.id, "error");
      }
    })
  );
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
  onDisable: () => void;
}

function ProviderCard({ state, onChange, onDisable }: ProviderCardProps) {
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
          onClick={onDisable}
          className="relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors bg-accent"
        >
          <span className="pointer-events-none inline-block h-5 w-5 rounded-full bg-surface shadow transform transition-transform translate-x-5" />
        </button>
      </div>

      <SensitiveInput
        label="API Key"
        value={state.api_key}
        onChange={(value) => onChange({ api_key: value, base_url: state.base_url, enabled: state.enabled })}
        placeholder="Enter API key"
      />

      <TextInput
        label="Base URL"
        value={state.base_url}
        onChange={(value) => onChange({ api_key: state.api_key, base_url: value || null, enabled: state.enabled })}
        placeholder="Optional custom base URL"
      />
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
      testAllProviders(providerStates, (id, status) => {
        setProviderStates((prev) =>
          prev.map((p) => (p.id === id ? { ...p, testStatus: status } : p))
        );
      }).finally(() => {
        testingRef.current = false;
      });
    }, 800);

    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
   
  }, [providerStates]);

  const configuredIds = Object.keys(providers);
  const unconfiguredIds = KNOWN_PROVIDERS.filter((id) => !configuredIds.includes(id));
  const sortedConfigured = KNOWN_PROVIDERS.filter((id) => id in providers);

  const updateProvider = (id: string, updated: ModelProviderConfig) => {
    setProviderStates((prev) =>
      prev.map((p) => (p.id === id ? { ...p, testStatus: "idle" as TestStatus } : p))
    );
    onChange({ ...providers, [id]: updated });
  };

  const enableProvider = (id: string) => {
    onChange({
      ...providers,
      [id]: { api_key: "", base_url: null, enabled: true },
    });
  };

  const disableProvider = (id: string) => {
    const next = { ...providers };
    delete next[id];
    onChange(next);
  };

  return (
    <div className="space-y-4">
      <SectionHeader title="Providers" description="Configure your LLM API providers" icon={CloudIcon} />
      {sortedConfigured.length > 0 && (
        <div className="space-y-3">
          {sortedConfigured.map((id) => {
            const state = providerStates.find((p) => p.id === id);
            if (!state) return null;
            return (
              <ProviderCard
                key={id}
                state={state}
                onChange={(updated) => updateProvider(id, updated)}
                onDisable={() => disableProvider(id)}
              />
            );
          })}
        </div>
      )}

      {unconfiguredIds.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium text-text-tertiary pt-2">Available Providers</h3>
          {unconfiguredIds.map((id) => (
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
