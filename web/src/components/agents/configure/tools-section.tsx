"use client";

import { useState, useEffect, useMemo } from "react";
import { SectionHeader, SectionPanel } from "@/components/settings/field";
import { WrenchScrewdriverIcon } from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import ReactMarkdown from "react-markdown";

interface ToolInfo {
  id: string;
  description: string;
  configurable: boolean;
}

type ToolProviderKind = { type: "builtin" } | { type: "mcp"; server_id: string; repository_url: string | null; version: string | null };
type ToolProviderStatus = { state: "available" } | { state: "unavailable"; reason: string };

interface ToolProviderWithTools {
  id: string;
  display_name: string;
  description: string | null;
  icon: string | null;
  kind: ToolProviderKind;
  status: ToolProviderStatus;
  tools: ToolInfo[];
}

interface ToolsSectionProps {
  tools: string[];
  onChange: (tools: string[]) => void;
}

/** Convert snake_case identifiers to Title Case for display (shell → Shell, web_fetch → Web Fetch). */
function titleCase(s: string): string {
  return s
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1).toLowerCase())
    .join(" ");
}

export function ToolsSection({ tools, onChange }: ToolsSectionProps) {
  const [providers, setProviders] = useState<ToolProviderWithTools[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [filters, setFilters] = useState<Record<string, string>>({});

  useEffect(() => {
    api.get<ToolProviderWithTools[]>("/api/tools").then(setProviders).catch(() => {});
  }, []);

  const sorted = useMemo(
    () =>
      providers
        .filter((p) => p.tools.length > 0)
        .sort((a, b) => {
          const aConfig = a.tools.some((t) => t.configurable);
          const bConfig = b.tools.some((t) => t.configurable);
          if (aConfig !== bConfig) return aConfig ? 1 : -1;
          return a.display_name.localeCompare(b.display_name);
        }),
    [providers],
  );

  const selected = useMemo(() => new Set(tools), [tools]);

  /** Returns true if a tool is effectively available to the agent:
   *  - non-configurable tools are always-on (registered unconditionally by the backend)
   *  - otherwise the tool id (or its provider id, for legacy entries) must be in the selection
   */
  const isToolSelected = (_provider: ToolProviderWithTools, toolId: string): boolean => {
    return selected.has(toolId);
  };

  /** all / none / partial */
  const providerSelectionState = (provider: ToolProviderWithTools): "all" | "none" | "partial" => {
    const allTools = provider.tools;
    if (allTools.length === 0) return "all";
    const selectedCount = allTools.filter((t) => isToolSelected(provider, t.id)).length;
    if (selectedCount === 0) return "none";
    if (selectedCount === allTools.length) return "all";
    return "partial";
  };

  /** Replace the user's selection with `next`, normalizing away legacy provider-id entries. */
  const updateSelection = (next: Set<string>) => {
    onChange(Array.from(next).sort());
  };

  const expandLegacyProviderIds = (set: Set<string>): Set<string> => {
    const result = new Set<string>();
    for (const entry of set) {
      const provider = providers.find((p) => p.id === entry);
      if (provider) {
        for (const tool of provider.tools) {
          result.add(tool.id);
        }
      } else {
        result.add(entry);
      }
    }
    return result;
  };

  const toggleProvider = (provider: ToolProviderWithTools) => {
    const state = providerSelectionState(provider);
    const next = expandLegacyProviderIds(selected);
    if (state === "all") {
      // remove every tool of this provider
      for (const t of provider.tools) next.delete(t.id);
    } else {
      // add every tool of this provider
      for (const t of provider.tools) {
        next.add(t.id);
      }
    }
    updateSelection(next);
  };

  const toggleTool = (provider: ToolProviderWithTools, toolId: string) => {
    const next = expandLegacyProviderIds(selected);
    if (next.has(toolId)) {
      next.delete(toolId);
    } else {
      next.add(toolId);
    }
    updateSelection(next);
  };

  const selectAllInProvider = (provider: ToolProviderWithTools) => {
    const next = expandLegacyProviderIds(selected);
    for (const t of provider.tools) {
      next.add(t.id);
    }
    updateSelection(next);
  };

  const clearProvider = (provider: ToolProviderWithTools) => {
    const next = expandLegacyProviderIds(selected);
    for (const t of provider.tools) next.delete(t.id);
    updateSelection(next);
  };

  const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div>
      <SectionHeader title="Tools" description="Tools available to this agent" icon={WrenchScrewdriverIcon} />
      <SectionPanel>
        <div className="space-y-3">
          {sorted.map((provider) => {
            const state = providerSelectionState(provider);
            const isExpanded = expanded.has(provider.id);
            const unavailable = provider.status.state === "unavailable";
            const anyConfigurable = provider.tools.some((t) => t.configurable);
            const filter = filters[provider.id] ?? "";
            const visibleTools = filter
              ? provider.tools.filter(
                  (t) =>
                    t.id.toLowerCase().includes(filter.toLowerCase()) ||
                    t.description.toLowerCase().includes(filter.toLowerCase()),
                )
              : provider.tools;

            // When a provider exposes exactly one tool, the provider checkbox directly
            // toggles that single tool; the expand panel only shows the description
            // (no per-tool checklist, since it would be redundant).
            const singleTool = provider.tools.length === 1 ? provider.tools[0] : null;

            return (
              <div
                key={provider.id}
                className={`rounded-lg border border-border bg-surface ${
                  unavailable || !anyConfigurable ? "opacity-60" : ""
                }`}
              >
                <div className="flex w-full items-center gap-3 px-4 py-3">
                  <input
                    type="checkbox"
                    checked={state === "all"}
                    ref={(el) => {
                      if (el) el.indeterminate = state === "partial";
                    }}
                    onChange={() => anyConfigurable && !unavailable && toggleProvider(provider)}
                    disabled={!anyConfigurable || unavailable}
                    className="h-4 w-4 rounded border-border text-accent focus:ring-accent disabled:opacity-50"
                  />
                  <button
                    type="button"
                    onClick={() => toggleExpand(provider.id)}
                    className="flex flex-1 items-center justify-between text-sm font-medium text-text-primary hover:text-text-secondary transition"
                    title={unavailable && provider.status.state === "unavailable" ? provider.status.reason : undefined}
                  >
                    <span className="flex items-center gap-2">
                      {provider.kind.type === "builtin"
                        ? titleCase(provider.display_name)
                        : provider.display_name.includes("/")
                          ? provider.display_name.split("/").pop()
                          : provider.display_name}
                      {unavailable && (
                        <span className="text-xs font-normal text-text-tertiary">(unavailable)</span>
                      )}
                      {!singleTool && provider.tools.length > 0 && (
                        <span className="text-xs font-normal text-text-tertiary">
                          {provider.tools.filter((t) => isToolSelected(provider, t.id)).length}/{provider.tools.length}
                        </span>
                      )}
                    </span>
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
                </div>
                {isExpanded && (
                  <div className="border-t border-border px-4 py-3 pl-11 space-y-3">
                    {provider.description && (
                      <div className="text-xs text-text-tertiary prose prose-xs prose-invert max-w-none">
                        <ReactMarkdown>{provider.description}</ReactMarkdown>
                      </div>
                    )}
                    {!singleTool && provider.tools.length > 5 && (
                      <input
                        type="text"
                        placeholder="Filter tools..."
                        value={filter}
                        onChange={(e) => setFilters((prev) => ({ ...prev, [provider.id]: e.target.value }))}
                        className="w-full rounded border border-border bg-background px-2 py-1 text-xs text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                      />
                    )}
                    {!singleTool && anyConfigurable && (
                      <div className="flex gap-2 text-xs">
                        <button
                          type="button"
                          onClick={() => selectAllInProvider(provider)}
                          disabled={unavailable}
                          className="text-accent hover:underline disabled:opacity-50"
                        >
                          Select all
                        </button>
                        <span className="text-text-tertiary">·</span>
                        <button
                          type="button"
                          onClick={() => clearProvider(provider)}
                          disabled={unavailable}
                          className="text-accent hover:underline disabled:opacity-50"
                        >
                          Clear
                        </button>
                      </div>
                    )}
                    {!singleTool && <div className="space-y-1">
                      {visibleTools.map((tool) => (
                        <label
                          key={tool.id}
                          className="flex items-start gap-2 text-xs text-text-secondary py-1"
                        >
                          <input
                            type="checkbox"
                            checked={isToolSelected(provider, tool.id)}
                            onChange={() => tool.configurable && !unavailable && toggleTool(provider, tool.id)}
                            disabled={!tool.configurable || unavailable}
                            className="h-3.5 w-3.5 mt-0.5 rounded border-border text-accent focus:ring-accent disabled:opacity-50"
                          />
                          <div className="flex-1">
                            <div className="font-mono text-text-primary">{tool.id.includes("__") ? tool.id.split("__").pop() : tool.id}</div>
                            {tool.description && (
                              <div className="text-text-tertiary line-clamp-2">{tool.description}</div>
                            )}
                          </div>
                        </label>
                      ))}
                      {visibleTools.length === 0 && (
                        <p className="text-xs text-text-tertiary italic">No tools match the filter.</p>
                      )}
                    </div>}
                  </div>
                )}
              </div>
            );
          })}
          {sorted.length === 0 && (
            <p className="text-sm text-text-tertiary">Loading tools...</p>
          )}
        </div>
      </SectionPanel>
    </div>
  );
}

