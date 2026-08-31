"use client";

import { useState, useEffect, useCallback } from "react";
import { ComboboxInput } from "@/components/settings/combobox";
import { getProviderModels } from "@/lib/config-types";
import type { ModelProviderConfig, ModelInfo } from "@/lib/config-types";

export type { ModelInfo };

interface ModelSelectorProps {
  label: string;
  provider: string;
  model: string;
  enabledProviders: string[];
  providerConfigs?: Record<string, ModelProviderConfig>;
  onProviderChange: (provider: string) => void;
  onModelChange: (model: string) => void;
  onModelInfo?: (info: ModelInfo | null) => void;
}

const PROVIDER_LABELS: Record<string, string> = {
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
  azure: "Azure OpenAI",
  zai: "Z.ai",
  venice: "Venice",
  minimax: "MiniMax",
  ollama: "Ollama",
  llamafile: "llamafile",
  generic: "OpenAI-compatible",
};

const modelsCache: Record<string, ModelInfo[]> = {};

export function ModelSelector({
  label,
  provider,
  model,
  enabledProviders,
  providerConfigs,
  onProviderChange,
  onModelChange,
  onModelInfo,
}: ModelSelectorProps) {
  const [availableModels, setAvailableModels] = useState<ModelInfo[]>(
    modelsCache[provider] ?? []
  );
  const [loading, setLoading] = useState(false);

  const fetchModels = useCallback(
    async (providerId: string) => {
      if (!providerId) {
        setAvailableModels([]);
        return;
      }
      if (modelsCache[providerId]) {
        setAvailableModels(modelsCache[providerId]);
        return;
      }
      setLoading(true);
      try {
        const cfg = providerConfigs?.[providerId];
        const apiKey = cfg?.api_key;
        const resp = await getProviderModels(providerId, {
          apiKey: typeof apiKey === "string" ? apiKey : undefined,
          baseUrl: cfg?.base_url ?? undefined,
        });
        modelsCache[providerId] = resp.models;
        setAvailableModels(resp.models);
      } catch {
        setAvailableModels([]);
      } finally {
        setLoading(false);
      }
    },
    [providerConfigs]
  );

  useEffect(() => {
    if (provider) {
      fetchModels(provider);
    }
  }, [provider, fetchModels]);

  useEffect(() => {
    if (onModelInfo && model && availableModels.length > 0) {
      // Exact ids only. A prefix match reports the first model that happens to
      // share a few characters, so half-typed input would pull that model's
      // token limits into the group.
      const info = availableModels.find((m) => m.id === model);
      onModelInfo(info ?? null);
    }
  }, [model, availableModels, onModelInfo]);

  const providerLabelById = new Map(enabledProviders.map((p) => [p, PROVIDER_LABELS[p] ?? p]));
  const providerIdByLabel = new Map(enabledProviders.map((p) => [PROVIDER_LABELS[p] ?? p, p]));
  const providerItems = enabledProviders.map((p) => {
    const display = PROVIDER_LABELS[p] ?? p;
    return { value: display, label: display };
  });
  const providerDisplay = providerLabelById.get(provider) ?? provider;

  const modelNameById = new Map(availableModels.map((m) => [m.id, m.name ?? m.id]));
  const modelIdByName = new Map(availableModels.map((m) => [m.name ?? m.id, m.id]));
  const modelItems = availableModels.map((m) => {
    const display = m.name ?? m.id;
    return { value: m.id, label: display };
  });
  // The combobox below is fully controlled by this value, so it has to echo
  // back exactly what the user typed. Resolving a friendly label for anything
  // but an exact id turned every keystroke into a prefix search whose first hit
  // replaced the input — and an empty field prefix-matched every model, so it
  // refilled itself the moment it was cleared.
  const modelDisplay = modelNameById.get(model) ?? model;

  return (
    <div className="space-y-2">
      {label && (
        <label className="block text-sm font-medium text-text-secondary">
          {label}
        </label>
      )}
      <div className="grid grid-cols-2 gap-2">
        <ComboboxInput
          label="Provider"
          value={providerDisplay}
          items={providerItems}
          placeholder="Select provider"
          allowFreeText
          onChange={(newDisplay) => {
            const newProvider = providerIdByLabel.get(newDisplay) ?? newDisplay;
            if (newProvider !== provider) {
              onProviderChange(newProvider);
              if (newProvider) fetchModels(newProvider);
            }
          }}
        />
        <ComboboxInput
          label={loading ? "Model (loading...)" : "Model"}
          value={modelDisplay}
          items={modelItems}
          placeholder={loading ? "Fetching models..." : "Select model"}
          allowFreeText
          disabled={!provider}
          onChange={(newValue) => {
            const modelId = modelIdByName.get(newValue) ?? newValue;
            onModelChange(modelId);
          }}
        />
      </div>
    </div>
  );
}
