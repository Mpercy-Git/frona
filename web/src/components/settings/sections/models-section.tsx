"use client";

import { useState, useEffect } from "react";
import type { ModelGroupConfig, RetryConfig } from "@/lib/config-types";
import { NumberInput, SectionHeader, Toggle } from "@/components/settings/field";
import { CubeIcon, Cog6ToothIcon } from "@heroicons/react/24/outline";
import { ComboboxInput } from "@/components/settings/combobox";
import { ModelSelector } from "@/components/settings/model-selector";
import { DeleteConfirmDialog } from "@/components/nav/delete-confirm-dialog";


interface ModelsSectionProps {
  models: Record<string, ModelGroupConfig>;
  enabledProviders: string[];
  providerConfigs?: Record<string, import("@/lib/config-types").ModelProviderConfig>;
  onChange: (models: Record<string, ModelGroupConfig>) => void;
}

const PREDEFINED_GROUPS = ["primary", "reasoning", "coding"];

function formatGroupName(name: string): string {
  const names: Record<string, string> = {
    primary: "Primary",
    reasoning: "Reasoning",
    coding: "Coding",
  };
  return names[name] ?? name;
}

function sortedGroupNames(names: string[]): string[] {
  const predefined = PREDEFINED_GROUPS.filter((g) => names.includes(g));
  const custom = names.filter((g) => !PREDEFINED_GROUPS.includes(g));
  return [...predefined, ...custom];
}

interface GroupNameInputProps {
  value: string;
  suggestions: string[];
  onRename: (newName: string) => void;
}

function GroupNameInput({ value, suggestions, onRename }: GroupNameInputProps) {
  const displayDraft = formatGroupName(value);
  const [draft, setDraft] = useState(displayDraft);

  const nameToId = new Map(suggestions.map((g) => [formatGroupName(g), g]));
  const items = suggestions.map((g) => {
    const display = formatGroupName(g);
    return { value: display, label: display };
  });

  return (
    <ComboboxInput
      label="Group ID"
      value={draft}
      items={items}
      placeholder="e.g. Primary, Coding"
      allowFreeText
      onChange={(v) => {
        setDraft(v);
        const id = nameToId.get(v);
        if (id) {
          onRename(id);
        }
      }}
      onBlur={() => {
        const resolved = nameToId.get(draft) ?? draft;
        const sanitized = resolved.trim().toLowerCase().replace(/\s+/g, "_").replace(/[^a-z0-9_]/g, "");
        if (sanitized && sanitized !== value) {
          setDraft(formatGroupName(sanitized));
          onRename(sanitized);
        } else {
          setDraft(formatGroupName(value));
        }
      }}
    />
  );
}

const DEFAULT_RETRY: RetryConfig = {
  max_retries: 10,
  initial_backoff_ms: 1000,
  backoff_multiplier: 2.0,
  max_backoff_ms: 60000,
};

function newModelGroup(): ModelGroupConfig {
  return {
    provider: "",
    model: "",
    fallbacks: [],
    max_tokens: null,
    temperature: null,
    context_window: null,
    retry: { ...DEFAULT_RETRY },
  };
}

function newFallback(): ModelGroupConfig {
  return {
    provider: "",
    model: "",
  };
}

interface ModelParamsDialogProps {
  group: ModelGroupConfig;
  groupName: string;
  onUpdate: (update: Partial<ModelGroupConfig>) => void;
  onClose: () => void;
}

function CollapsibleSection({ title, defaultOpen = false, children }: { title: string; defaultOpen?: boolean; children: React.ReactNode }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="border-b border-border last:border-b-0">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between py-3 text-sm font-medium text-text-secondary hover:text-text-primary transition"
      >
        <span>{title}</span>
        <svg
          className={`h-4 w-4 text-text-tertiary transition-transform ${open ? "rotate-90" : ""}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
        </svg>
      </button>
      {open && <div className="pb-4 space-y-4">{children}</div>}
    </div>
  );
}

function ModelParamsDialog({ group, groupName, onUpdate, onClose }: ModelParamsDialogProps) {
  const hasProviderParams = !!group.provider && group.provider !== "generic";

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [onClose]);

  const retry = group.retry ?? DEFAULT_RETRY;

  function updateRetry(update: Partial<RetryConfig>) {
    onUpdate({ retry: { ...retry, ...update } });
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      <div className="relative rounded-xl border border-border bg-surface-secondary p-4 max-w-lg w-full mx-4 shadow-xl max-h-[85vh] flex flex-col">
        <div className="pb-3 border-b border-border flex items-start justify-between gap-3 -mx-4 px-4">
          <div>
            <h3 className="text-lg font-semibold text-text-primary">
              {groupName}
            </h3>
            <p className="text-sm text-text-tertiary mt-1">Model parameters</p>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-text-tertiary hover:bg-surface-tertiary hover:text-text-primary transition shrink-0"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="overflow-y-auto mt-1">
          <CollapsibleSection title="General" defaultOpen>
            <div className="grid grid-cols-2 gap-4">
              <NumberInput
                label="Max Tokens"
                value={group.max_tokens ?? null}
                onChange={(v) => onUpdate({ max_tokens: v || null })}
                min={1}
                placeholder="Default"
              />
              <NumberInput
                label="Context Window"
                value={group.context_window ?? null}
                onChange={(v) => onUpdate({ context_window: v || null })}
                min={1}
                placeholder="Default"
              />
            </div>
            <div className="space-y-1">
              <div className="flex items-center justify-between">
                <label className="block text-sm font-medium text-text-secondary">Temperature</label>
                <span className="text-xs text-text-tertiary tabular-nums">
                  {group.temperature != null ? group.temperature.toFixed(1) : "Default"}
                </span>
              </div>
              <input
                type="range"
                min={0}
                max={2}
                step={0.1}
                value={group.temperature ?? 0}
                onChange={(e) => {
                  const v = parseFloat(e.target.value);
                  onUpdate({ temperature: v === 0 ? null : v });
                }}
                className="w-full accent-accent"
              />
            </div>
          </CollapsibleSection>

          {hasProviderParams && (
            <CollapsibleSection title={group.provider.charAt(0).toUpperCase() + group.provider.slice(1)}>
              <ProviderParams group={group} onUpdate={onUpdate} />
            </CollapsibleSection>
          )}

          <CollapsibleSection title="Retry">
            <div className="grid grid-cols-2 gap-4">
              <NumberInput
                label="Max Retries"
                value={retry.max_retries}
                onChange={(v) => updateRetry({ max_retries: v })}
                min={0}
              />
              <NumberInput
                label="Initial Backoff (ms)"
                value={retry.initial_backoff_ms}
                onChange={(v) => updateRetry({ initial_backoff_ms: v })}
                min={0}
              />
              <NumberInput
                label="Backoff Multiplier"
                value={retry.backoff_multiplier}
                onChange={(v) => updateRetry({ backoff_multiplier: v })}
                min={1}
                step={0.1}
              />
              <NumberInput
                label="Max Backoff (ms)"
                value={retry.max_backoff_ms}
                onChange={(v) => updateRetry({ max_backoff_ms: v })}
                min={0}
              />
            </div>
          </CollapsibleSection>
        </div>
      </div>
    </div>
  );
}

function ProviderParams({ group, onUpdate }: { group: ModelGroupConfig; onUpdate: (u: Partial<ModelGroupConfig>) => void }) {
  switch (group.provider) {
    case "anthropic":
      return <AnthropicParams group={group} onUpdate={onUpdate} />;
    case "ollama":
      return <OllamaParams group={group} onUpdate={onUpdate} />;
    case "openai":
    case "groq":
    case "openrouter":
    case "deepseek":
    case "xai":
    case "together":
    case "hyperbolic":
      return <OpenAIParams group={group} onUpdate={onUpdate} />;
    case "gemini":
      return <GeminiParams group={group} onUpdate={onUpdate} />;
    default:
      return <p className="text-sm text-text-tertiary">No provider-specific parameters.</p>;
  }
}

function AnthropicParams({ group, onUpdate }: { group: ModelGroupConfig; onUpdate: (u: Partial<ModelGroupConfig>) => void }) {
  const thinkingEnabled = group.thinking?.type === "enabled";
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-4 items-start">
        <Toggle
          label="Extended Thinking"
          value={thinkingEnabled}
          onChange={(v) => onUpdate({
            thinking: v ? { type: "enabled", budget_tokens: group.thinking?.budget_tokens ?? 16000 } : { type: "disabled" },
          })}
        />
        {thinkingEnabled && (
          <NumberInput
            label="Budget (tokens)"
            value={group.thinking?.budget_tokens ?? null}
            onChange={(v) => onUpdate({
              thinking: { type: "enabled", budget_tokens: v || undefined },
            })}
            min={1}
            placeholder="16000"
          />
        )}
      </div>
      <div className="grid grid-cols-2 gap-4">
        <NumberInput label="Top P" value={group.top_p ?? null} onChange={(v) => onUpdate({ top_p: v || null })} min={0} step={0.05} placeholder="Default" />
        <NumberInput label="Top K" value={group.top_k ?? null} onChange={(v) => onUpdate({ top_k: v || null })} min={0} placeholder="Default" />
      </div>
    </div>
  );
}

function OllamaParams({ group, onUpdate }: { group: ModelGroupConfig; onUpdate: (u: Partial<ModelGroupConfig>) => void }) {
  return (
    <div className="space-y-4">
      <Toggle label="Think" value={group.think ?? false} onChange={(v) => onUpdate({ think: v || null })} />
      <div className="grid grid-cols-2 gap-4">
        <NumberInput label="Context Size (num_ctx)" value={group.num_ctx ?? null} onChange={(v) => onUpdate({ num_ctx: v || null })} min={1} placeholder="Default" />
        <NumberInput label="Max Predict (num_predict)" value={group.num_predict ?? null} onChange={(v) => onUpdate({ num_predict: v || null })} min={1} placeholder="Default" />
        <NumberInput label="Top K" value={group.top_k ?? null} onChange={(v) => onUpdate({ top_k: v || null })} min={0} placeholder="40" />
        <NumberInput label="Top P" value={group.top_p ?? null} onChange={(v) => onUpdate({ top_p: v || null })} min={0} step={0.05} placeholder="0.9" />
        <NumberInput label="Min P" value={group.min_p ?? null} onChange={(v) => onUpdate({ min_p: v || null })} min={0} step={0.05} placeholder="0.0" />
        <NumberInput label="Repeat Penalty" value={group.repeat_penalty ?? null} onChange={(v) => onUpdate({ repeat_penalty: v || null })} min={0} step={0.05} placeholder="1.1" />
        <NumberInput label="Repeat Last N" value={group.repeat_last_n ?? null} onChange={(v) => onUpdate({ repeat_last_n: v || null })} min={-1} placeholder="64" />
        <NumberInput label="Seed" value={group.seed ?? null} onChange={(v) => onUpdate({ seed: v || null })} min={0} placeholder="Random" />
        <NumberInput label="Mirostat" value={group.mirostat ?? null} onChange={(v) => onUpdate({ mirostat: v || null })} min={0} placeholder="0" />
        <NumberInput label="Mirostat Eta" value={group.mirostat_eta ?? null} onChange={(v) => onUpdate({ mirostat_eta: v || null })} min={0} step={0.05} placeholder="0.1" />
        <NumberInput label="Mirostat Tau" value={group.mirostat_tau ?? null} onChange={(v) => onUpdate({ mirostat_tau: v || null })} min={0} step={0.5} placeholder="5.0" />
        <NumberInput label="Num GPU" value={group.num_gpu ?? null} onChange={(v) => onUpdate({ num_gpu: v || null })} min={0} placeholder="Default" />
        <NumberInput label="Num Thread" value={group.num_thread ?? null} onChange={(v) => onUpdate({ num_thread: v || null })} min={1} placeholder="Default" />
        <NumberInput label="Num Batch" value={group.num_batch ?? null} onChange={(v) => onUpdate({ num_batch: v || null })} min={1} placeholder="512" />
      </div>
      <div className="grid grid-cols-2 gap-4">
        <Toggle label="Use MMap" value={group.use_mmap ?? false} onChange={(v) => onUpdate({ use_mmap: v || null })} />
        <Toggle label="Use MLock" value={group.use_mlock ?? false} onChange={(v) => onUpdate({ use_mlock: v || null })} />
      </div>
    </div>
  );
}

function OpenAIParams({ group, onUpdate }: { group: ModelGroupConfig; onUpdate: (u: Partial<ModelGroupConfig>) => void }) {
  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <label className="block text-sm font-medium text-text-secondary">Reasoning Effort</label>
        <select
          value={group.reasoning_effort ?? ""}
          onChange={(e) => onUpdate({ reasoning_effort: e.target.value || null })}
          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
        >
          <option value="">Default</option>
          <option value="low">Low</option>
          <option value="medium">Medium</option>
          <option value="high">High</option>
        </select>
      </div>
      <div className="grid grid-cols-2 gap-4">
        <NumberInput label="Top P" value={group.top_p ?? null} onChange={(v) => onUpdate({ top_p: v || null })} min={0} step={0.05} placeholder="Default" />
        <NumberInput label="Min P" value={group.min_p ?? null} onChange={(v) => onUpdate({ min_p: v || null })} min={0} step={0.05} placeholder="Default" />
        <NumberInput label="Frequency Penalty" value={group.frequency_penalty ?? null} onChange={(v) => onUpdate({ frequency_penalty: v || null })} step={0.05} placeholder="0.0" />
        <NumberInput label="Presence Penalty" value={group.presence_penalty ?? null} onChange={(v) => onUpdate({ presence_penalty: v || null })} step={0.05} placeholder="0.0" />
        <NumberInput label="Seed" value={group.seed ?? null} onChange={(v) => onUpdate({ seed: v || null })} min={0} placeholder="Random" />
        <NumberInput label="Max Completion Tokens" value={group.max_completion_tokens ?? null} onChange={(v) => onUpdate({ max_completion_tokens: v || null })} min={1} placeholder="Default" />
      </div>
      <Toggle label="Log Probabilities" value={group.logprobs ?? false} onChange={(v) => onUpdate({ logprobs: v || null })} />
    </div>
  );
}

function GeminiParams({ group, onUpdate }: { group: ModelGroupConfig; onUpdate: (u: Partial<ModelGroupConfig>) => void }) {
  const thinkingEnabled = !!group.thinking_config;
  return (
    <div className="space-y-4">
      <Toggle
        label="Thinking"
        value={thinkingEnabled}
        onChange={(v) => onUpdate({
          thinking_config: v ? { thinking_budget: group.thinking_config?.thinking_budget ?? 8192 } : null,
        })}
      />
      {thinkingEnabled && (
        <NumberInput
          label="Thinking Budget (tokens)"
          value={group.thinking_config?.thinking_budget ?? null}
          onChange={(v) => onUpdate({
            thinking_config: v ? { thinking_budget: v } : null,
          })}
          min={1}
          placeholder="8192"
        />
      )}
      <div className="grid grid-cols-2 gap-4">
        <NumberInput label="Top P" value={group.top_p ?? null} onChange={(v) => onUpdate({ top_p: v || null })} min={0} step={0.05} placeholder="Default" />
        <NumberInput label="Top K" value={group.top_k ?? null} onChange={(v) => onUpdate({ top_k: v || null })} min={0} placeholder="Default" />
        <NumberInput label="Candidate Count" value={group.candidate_count ?? null} onChange={(v) => onUpdate({ candidate_count: v || null })} min={1} placeholder="1" />
      </div>
    </div>
  );
}

export function ModelsSection({ models, enabledProviders, providerConfigs, onChange }: ModelsSectionProps) {
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [paramsDialog, setParamsDialog] = useState<{ group: string; fallbackIndex?: number } | null>(null);
  const [confirmingRemove, setConfirmingRemove] = useState<string | null>(null);

  const groupNames = sortedGroupNames(Object.keys(models));

  function toggleExpanded(name: string) {
    setExpandedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  function updateGroup(name: string, update: Partial<ModelGroupConfig>) {
    onChange({ ...models, [name]: { ...models[name], ...update } });
  }

  function renameGroup(oldName: string, newName: string) {
    if (!newName.trim() || (newName !== oldName && newName in models)) return;
    const entries = Object.entries(models).map(([k, v]) =>
      k === oldName ? [newName, v] : [k, v]
    );
    onChange(Object.fromEntries(entries));
    setExpandedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(oldName)) {
        next.delete(oldName);
        next.add(newName);
      }
      return next;
    });
  }

  function removeGroup(name: string) {
    const next = { ...models };
    delete next[name];
    onChange(next);
    setConfirmingRemove(null);
  }

  function addGroup() {
    let name = "";
    let i = 1;
    while (name in models) {
      name = `group_${i++}`;
    }
    onChange({ ...models, [name]: newModelGroup() });
    setExpandedGroups((prev) => new Set(prev).add(name));
  }

  function updateFallback(groupName: string, index: number, update: Partial<ModelGroupConfig>) {
    const fallbacks = [...(models[groupName].fallbacks ?? [])];
    fallbacks[index] = { ...fallbacks[index], ...update };
    updateGroup(groupName, { fallbacks });
  }

  function addFallback(groupName: string) {
    updateGroup(groupName, { fallbacks: [...(models[groupName].fallbacks ?? []), newFallback()] });
  }

  function removeFallback(groupName: string, index: number) {
    const fallbacks = (models[groupName].fallbacks ?? []).filter((_, i) => i !== index);
    updateGroup(groupName, { fallbacks });
  }

  return (
    <div>
      <SectionHeader title="Model Groups" description="Configure model groups with fallback chains and inference parameters" icon={CubeIcon} />
      <div className="space-y-3">
        {groupNames.map((name) => {
          const group = models[name];
          const isExpanded = expandedGroups.has(name);

          return (
            <div
              key={name}
              className="rounded-lg border border-border bg-surface-secondary"
            >
              <button
                type="button"
                onClick={() => toggleExpanded(name)}
                className="flex w-full items-center justify-between px-4 py-3 text-sm font-medium text-text-primary hover:bg-surface-tertiary rounded-lg"
              >
                <span>{formatGroupName(name)}</span>
                <svg
                  className={`h-4 w-4 text-text-tertiary transition-transform ${isExpanded ? "rotate-180" : ""}`}
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}
                >
                  <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
                </svg>
              </button>

              {isExpanded && (
                <div className="space-y-4 px-4 pb-4">
                  <GroupNameInput
                    value={name}
                    suggestions={PREDEFINED_GROUPS.filter((g) => g === name || !(g in models))}
                    onRename={(newName) => renameGroup(name, newName)}
                  />

                  <div className="flex items-end gap-2">
                    <div className="flex-1">
                      <ModelSelector
                        label="Main Model"
                        provider={group.provider}
                        model={group.model}
                        enabledProviders={enabledProviders}
                        providerConfigs={providerConfigs}
                        onProviderChange={(v) => updateGroup(name, { provider: v, model: "" })}
                        onModelChange={(v) => updateGroup(name, { model: v })}
                        onModelInfo={(info) => {
                          if (info) {
                            const update: Partial<ModelGroupConfig> = {};
                            if (info.max_tokens && info.max_tokens !== group.max_tokens) update.max_tokens = info.max_tokens;
                            if (info.context_window && info.context_window !== group.context_window) update.context_window = info.context_window;
                            if (Object.keys(update).length > 0) updateGroup(name, update);
                          }
                        }}
                      />
                    </div>
                    <button
                      type="button"
                      onClick={() => setParamsDialog({ group: name })}
                      disabled={!group.provider || !group.model}
                      className="shrink-0 h-[38px] rounded-lg border border-border bg-surface px-2.5 text-text-tertiary hover:bg-surface-tertiary hover:text-text-primary transition disabled:opacity-30 disabled:pointer-events-none"
                      title="Parameters"
                    >
                      <Cog6ToothIcon className="h-4 w-4" />
                    </button>
                  </div>

                  {(group.fallbacks ?? []).length > 0 && (
                  <div className="space-y-2">
                    <label className="block text-sm font-medium text-text-secondary">
                      Fallbacks
                    </label>
                    {(group.fallbacks ?? []).map((fb, i) => (
                      <div key={i} className="flex items-end gap-2">
                        <span className="text-xs text-text-tertiary w-5 text-right shrink-0 pb-2.5">
                          {i + 1}.
                        </span>
                        <div className="flex-1">
                          <ModelSelector
                            label=""
                            provider={fb.provider}
                            model={fb.model}
                            enabledProviders={enabledProviders}
                            providerConfigs={providerConfigs}
                            onProviderChange={(v) => updateFallback(name, i, { provider: v, model: "" })}
                            onModelChange={(v) => updateFallback(name, i, { model: v })}
                          />
                        </div>
                        <button
                          type="button"
                          onClick={() => setParamsDialog({ group: name, fallbackIndex: i })}
                          disabled={!fb.provider || !fb.model}
                          className="shrink-0 h-[38px] rounded-lg border border-border bg-surface px-2.5 text-text-tertiary hover:bg-surface-tertiary hover:text-text-primary transition disabled:opacity-30 disabled:pointer-events-none"
                          title="Parameters"
                        >
                          <Cog6ToothIcon className="h-4 w-4" />
                        </button>
                        <button
                          type="button"
                          onClick={() => removeFallback(name, i)}
                          className="shrink-0 h-[38px] rounded-lg p-1.5 text-text-tertiary hover:bg-surface-tertiary hover:text-text-primary flex items-center"
                        >
                          <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                          </svg>
                        </button>
                      </div>
                    ))}
                  </div>
                  )}

                  <div className="flex items-center gap-2 pt-1">
                    <button
                      type="button"
                      onClick={() => addFallback(name)}
                      className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-text-secondary hover:bg-surface-tertiary transition flex items-center gap-1.5"
                    >
                      <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
                      </svg>
                      Fallback
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirmingRemove(name)}
                      className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-danger hover:bg-surface-tertiary transition ml-auto"
                    >
                      Remove
                    </button>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>

      <button
        type="button"
        onClick={addGroup}
        className="mt-4 rounded-lg bg-accent px-4 py-2 text-sm text-white hover:opacity-90"
      >
        + Add Model Group
      </button>

      {paramsDialog && (() => {
        const { group: groupName, fallbackIndex } = paramsDialog;
        const groupConfig = models[groupName];
        if (!groupConfig) return null;
        const isFallback = fallbackIndex != null;
        const target = isFallback ? (groupConfig.fallbacks ?? [])[fallbackIndex] : groupConfig;
        if (!target) return null;
        const label = isFallback
          ? `${formatGroupName(groupName)} — Fallback ${fallbackIndex + 1}`
          : formatGroupName(groupName);
        return (
          <ModelParamsDialog
            group={target}
            groupName={label}
            onUpdate={(update) => {
              if (isFallback) {
                updateFallback(groupName, fallbackIndex, update);
              } else {
                updateGroup(groupName, update);
              }
            }}
            onClose={() => setParamsDialog(null)}
          />
        );
      })()}

      <DeleteConfirmDialog
        open={!!confirmingRemove}
        onCancel={() => setConfirmingRemove(null)}
        onConfirm={() => { if (confirmingRemove) removeGroup(confirmingRemove); }}
        title={`Remove ${confirmingRemove ? formatGroupName(confirmingRemove) : ""}?`}
        message="This model group and its configuration will be removed."
      />
    </div>
  );
}
