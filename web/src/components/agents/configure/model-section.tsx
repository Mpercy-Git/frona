"use client";

import { useState, useEffect } from "react";
import { Field, SectionHeader, SectionPanel, SelectInput } from "@/components/settings/field";
import { CpuChipIcon } from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import type { ModelGroupConfig } from "@/lib/config-types";

interface ModelSectionProps {
  modelGroup: string;
  onModelGroupChange: (modelGroup: string) => void;
}

function formatGroupName(name: string): string {
  return name.split("_").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
}

export function ModelSection({ modelGroup, onModelGroupChange }: ModelSectionProps) {
  const [groupOptions, setGroupOptions] = useState<{ value: string; label: string }[]>([]);
  const [models, setModels] = useState<Record<string, ModelGroupConfig>>({});

  useEffect(() => {
    api.get<{ models: Record<string, ModelGroupConfig> }>("/api/config")
      .then((config) => {
        setModels(config.models);
        setGroupOptions(
          Object.keys(config.models).map((name) => ({ value: name, label: formatGroupName(name) }))
        );
      })
      .catch(() => {});
  }, []);

  const group = models[modelGroup];

  return (
    <div>
      <SectionHeader title="Model" description="Language model configuration" icon={CpuChipIcon} />
      <SectionPanel>
        <p className="text-sm text-text-tertiary">
          Select a model group to control which language model powers this agent, including its fallback chain, response quality, speed, and cost.
        </p>
        <SelectInput
          label="Model Group"
          value={modelGroup}
          onChange={(v) => onModelGroupChange(v ?? "primary")}
          options={groupOptions}
          allowEmpty={false}
        />
        {group && (
          <div className="grid grid-cols-2 gap-4">
            <Field label="Provider">
              <p className="text-sm text-text-primary">{group.provider || "—"}</p>
            </Field>
            <Field label="Model">
              <p className="text-sm text-text-primary font-mono">{group.model || "—"}</p>
            </Field>
            {(group.fallbacks ?? []).length > 0 && (
              <Field label="Fallbacks">
                {(group.fallbacks ?? []).map((fb, i) => (
                  <p key={i} className="text-sm text-text-primary font-mono">
                    {fb.provider}/{fb.model}
                  </p>
                ))}
              </Field>
            )}
            {group.temperature != null && (
              <Field label="Temperature">
                <p className="text-sm text-text-primary">{group.temperature}</p>
              </Field>
            )}
            {group.max_tokens != null && (
              <Field label="Max Tokens">
                <p className="text-sm text-text-primary">{group.max_tokens.toLocaleString()}</p>
              </Field>
            )}
            {group.context_window != null && (
              <Field label="Context Window">
                <p className="text-sm text-text-primary">{group.context_window.toLocaleString()}</p>
              </Field>
            )}
          </div>
        )}
      </SectionPanel>
    </div>
  );
}
