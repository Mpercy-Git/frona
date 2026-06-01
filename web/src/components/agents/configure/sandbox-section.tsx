"use client";

import { useState, useEffect } from "react";
import { useAuth } from "@/lib/auth";
import { api } from "@/lib/api-client";
import { getConfig, type SandboxConfig } from "@/lib/config-types";
import { SectionHeader, SectionPanel, Toggle, Field, HelpTip } from "@/components/settings/field";
import { ShieldCheckIcon, TrashIcon, PlusIcon, FolderOpenIcon, UserIcon, CpuChipIcon } from "@heroicons/react/24/outline";
import { FileBrowserModal } from "@/components/chat/file-browser-modal";
import type { Agent, Attachment } from "@/lib/types";

interface SystemInfo {
  cpus: number;
  total_memory_bytes: number;
  sandbox_driver: string;
}

interface SharedPath {
  path: string;
  write: boolean;
}

interface SandboxSettings {
  network_access: boolean;
  allowed_network_destinations: string[];
  timeout_secs?: number;
  max_cpu_pct?: number;
  max_memory_pct?: number;
  shared_paths: SharedPath[];
}

interface SandboxSectionProps {
  sandbox: SandboxSettings | null;
  onChange: (sandbox: SandboxSettings) => void;
  onValidChange?: (valid: boolean) => void;
}

const DEFAULT_SANDBOX: SandboxSettings = {
  network_access: true,
  allowed_network_destinations: [],
  shared_paths: [],
};

function formatBytes(bytes: number): string {
  const gb = bytes / 1_073_741_824;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / 1_048_576).toFixed(0)} MB`;
}

type Virtual = { kind: "user" | "agent"; owner: string; relative: string };

function parseVirtual(path: string): Virtual | null {
  for (const kind of ["user", "agent"] as const) {
    const prefix = `${kind}://`;
    if (path.startsWith(prefix)) {
      const rest = path.slice(prefix.length);
      const slash = rest.indexOf("/");
      const owner = slash === -1 ? rest : rest.slice(0, slash);
      const relative = slash === -1 ? "" : rest.slice(slash + 1);
      return { kind, owner, relative };
    }
  }
  return null;
}

function buildVirtual(v: Virtual): string {
  return `${v.kind}://${v.owner}/${v.relative}`;
}

function isValidDest(value: string): boolean {
  if (!value) return true;
  const hostname = /^[a-zA-Z0-9]([a-zA-Z0-9.-]*[a-zA-Z0-9])?$/;
  const ipv4 = /^(\d{1,3}\.){3}\d{1,3}(\/\d{1,2})?$/;
  const ipv6 = /^[0-9a-fA-F:]+(::\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})?(\/\d{1,3})?$/;
  return hostname.test(value) || ipv4.test(value) || ipv6.test(value);
}

export function SandboxSection({ sandbox, onChange, onValidChange }: SandboxSectionProps) {
  const { user } = useAuth();
  const current = sandbox ?? DEFAULT_SANDBOX;
  const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
  const [serverConfig, setServerConfig] = useState<SandboxConfig | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);

  useEffect(() => {
    api.get<SystemInfo>("/api/system/info").then(setSysInfo).catch(() => {});
    getConfig().then((c) => setServerConfig(c.sandbox)).catch(() => {});
    api.get<Agent[]>("/api/agents").then(setAgents).catch(() => {});
  }, []);

  const agentNameById = (id: string): string =>
    agents.find((a) => a.id === id)?.name ?? id;

  const driver = sysInfo?.sandbox_driver ?? "disabled";
  const supportsNetworkDestinations = driver === "macos" || driver === "syd";
  const sandboxEnabled = driver !== "disabled";

  const hasInvalidDests = current.allowed_network_destinations.some((d) => d !== "" && !isValidDest(d));

  useEffect(() => {
    onValidChange?.(!hasInvalidDests);
  }, [hasInvalidDests, onValidChange]);

  const updateDest = (index: number, value: string) => {
    const updated = [...current.allowed_network_destinations];
    updated[index] = value;
    onChange({ ...current, allowed_network_destinations: updated });
  };

  const removeDest = (index: number) => {
    onChange({
      ...current,
      allowed_network_destinations: current.allowed_network_destinations.filter((_, i) => i !== index),
    });
  };

  const addDest = () => {
    onChange({
      ...current,
      allowed_network_destinations: [...current.allowed_network_destinations, ""],
    });
  };

  const updateSharedPath = (index: number, value: string) => {
    const updated = [...current.shared_paths];
    updated[index] = { ...updated[index], path: value };
    onChange({ ...current, shared_paths: updated });
  };

  const updateSharedRelative = (index: number, relative: string) => {
    const entry = current.shared_paths[index];
    const v = parseVirtual(entry.path);
    if (!v) return;
    const next = buildVirtual({ ...v, relative: relative.replace(/^\/+/, "") });
    updateSharedPath(index, next);
  };

  const toggleSharedWrite = (index: number, write: boolean) => {
    const updated = [...current.shared_paths];
    updated[index] = { ...updated[index], write };
    onChange({ ...current, shared_paths: updated });
  };

  const removeSharedPath = (index: number) => {
    onChange({
      ...current,
      shared_paths: current.shared_paths.filter((_, i) => i !== index),
    });
  };

  const addSharedPath = () => {
    onChange({
      ...current,
      shared_paths: [...current.shared_paths, { path: "", write: false }],
    });
  };

  const [browseOpen, setBrowseOpen] = useState(false);

  const handleBrowseSelect = (attachments: Attachment[]) => {
    const paths = attachments.map((a) => {
      const [kind, id] = a.owner.split(":");
      if (kind === "agent") return `agent://${id}/${a.path}`;
      return `user://${user?.handle ?? id}/${a.path}`;
    });
    const existingPaths = new Set(current.shared_paths.map((e) => e.path));
    const additions = paths
      .filter((p) => !existingPaths.has(p))
      .map((path) => ({ path, write: false }));
    if (additions.length > 0) {
      onChange({ ...current, shared_paths: [...current.shared_paths, ...additions] });
    }
    setBrowseOpen(false);
  };

  return (
    <div className="space-y-4">
      <SectionHeader title="Sandbox" description="Execution environment constraints" icon={ShieldCheckIcon} />

      <SectionPanel title="Resources">
        <div className="grid grid-cols-2 gap-x-6">
          <Field label="Max Memory (%)" description="Kill process if agent memory exceeds this. Leave empty for server default.">
            <div className="flex items-center gap-3">
              <input
                type="number"
                value={current.max_memory_pct ?? ""}
                onChange={(e) => {
                  const v = e.target.value ? Math.min(100, Math.max(1, Number(e.target.value))) : undefined;
                  onChange({ ...current, max_memory_pct: v });
                }}
                min={1}
                max={100}
                placeholder={serverConfig ? String(serverConfig.max_memory_pct) : ""}
                className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {sysInfo && serverConfig && (
                <span className="text-xs text-text-tertiary">
                  {formatBytes(((current.max_memory_pct ?? serverConfig.max_memory_pct) / 100) * sysInfo.total_memory_bytes)} of {formatBytes(sysInfo.total_memory_bytes)}
                </span>
              )}
            </div>
          </Field>
          <Field label="Max CPU (%)" description="Kill process if agent CPU exceeds this. Leave empty for server default.">
            <div className="flex items-center gap-3">
              <input
                type="number"
                value={current.max_cpu_pct ?? ""}
                onChange={(e) => {
                  const v = e.target.value ? Math.min(100, Math.max(1, Number(e.target.value))) : undefined;
                  onChange({ ...current, max_cpu_pct: v });
                }}
                min={1}
                max={100}
                placeholder={serverConfig ? String(serverConfig.max_cpu_pct) : ""}
                className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {sysInfo && serverConfig && (
                <span className="text-xs text-text-tertiary">
                  {parseFloat((((current.max_cpu_pct ?? serverConfig.max_cpu_pct) / 100) * sysInfo.cpus).toFixed(1))} of {sysInfo.cpus} cores
                </span>
              )}
            </div>
          </Field>
        </div>
        <Field label="Timeout (seconds)" description="Maximum execution time for tool calls. Leave empty to use the server default.">
          <input
            type="number"
            value={current.timeout_secs ?? ""}
            onChange={(e) => {
              const v = e.target.value ? Math.max(0, Number(e.target.value)) : undefined;
              onChange({ ...current, timeout_secs: v });
            }}
            min={0}
            max={300}
            placeholder={serverConfig ? String(serverConfig.timeout_secs) : ""}
            className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
          />
        </Field>
      </SectionPanel>

      {sandboxEnabled && (
        <SectionPanel title="Network">
          <Toggle
            label="Network Access"
            description="Allow the agent to make network requests"
            value={current.network_access}
            onChange={(v) => onChange({ ...current, network_access: v })}
          />
          {supportsNetworkDestinations && (
            <div className="space-y-1">
              <label className="inline-flex items-center gap-1 text-sm font-medium text-text-secondary">
                Allowed Destinations
                <HelpTip content="Restrict network access to specific destinations. Supports hostnames (www.cloudflare.com), IPs (1.1.1.1), and CIDR ranges (10.10.100.1/24). Leave empty to allow all destinations when network access is enabled." />
              </label>
              <div className="grid grid-cols-2 gap-2">
                {current.allowed_network_destinations.map((dest, i) => (
                  <div key={i} className={`flex items-center gap-1.5 rounded-lg border bg-surface px-2 py-1.5 ${dest && !isValidDest(dest) ? "border-red-500" : "border-border"}`}>
                    <input
                      value={dest}
                      onChange={(e) => updateDest(i, e.target.value)}
                      placeholder="e.g. api.example.com"
                      className="flex-1 min-w-0 bg-transparent text-sm text-text-primary placeholder:text-text-tertiary focus:outline-none"
                    />
                    <button
                      onClick={() => removeDest(i)}
                      className="shrink-0 rounded p-0.5 text-text-tertiary hover:text-text-primary transition"
                    >
                      <TrashIcon className="h-3.5 w-3.5" />
                    </button>
                  </div>
                ))}
              </div>
              <button
                onClick={addDest}
                className="flex items-center gap-1 text-sm text-accent hover:underline mt-2"
              >
                <PlusIcon className="h-4 w-4" />
                Add destination
              </button>
            </div>
          )}
        </SectionPanel>
      )}

      {sandboxEnabled && (
        <SectionPanel title="Shared Files" helpTip="Files or directories the agent can read inside the sandbox. Toggle Write to also grant write access.">
          <div className="space-y-1">
            <div className="grid grid-cols-2 gap-2">
              {current.shared_paths.map((entry, i) => {
                const v = parseVirtual(entry.path);
                const ownerLabel = v
                  ? v.kind === "agent"
                    ? agentNameById(v.owner)
                    : v.owner
                  : null;
                const OwnerIcon = v?.kind === "agent" ? CpuChipIcon : UserIcon;
                return (
                  <div key={i} className="flex items-center gap-1.5 rounded-lg border border-border bg-surface px-2 py-1.5">
                    {v ? (
                      <>
                        <span
                          className="flex shrink-0 items-center gap-1 rounded bg-surface-tertiary px-1.5 py-0.5 text-xs text-text-secondary"
                          title={`${v.kind}://${v.owner}/`}
                        >
                          <OwnerIcon className="h-3 w-3" />
                          {ownerLabel}
                        </span>
                        <span className="text-text-tertiary text-sm select-none">/</span>
                        <input
                          value={v.relative}
                          onChange={(e) => updateSharedRelative(i, e.target.value)}
                          placeholder="path/within"
                          className="flex-1 min-w-0 bg-transparent text-sm text-text-primary placeholder:text-text-tertiary focus:outline-none"
                        />
                      </>
                    ) : (
                      <input
                        value={entry.path}
                        onChange={(e) => updateSharedPath(i, e.target.value)}
                        placeholder="e.g. /data/shared"
                        className="flex-1 min-w-0 bg-transparent text-sm text-text-primary placeholder:text-text-tertiary focus:outline-none"
                      />
                    )}
                    <label className="flex shrink-0 cursor-pointer items-center gap-1 text-xs text-text-secondary select-none">
                      <input
                        type="checkbox"
                        checked={entry.write}
                        onChange={(e) => toggleSharedWrite(i, e.target.checked)}
                        className="h-3.5 w-3.5 accent-accent"
                      />
                      Write
                    </label>
                    <button
                      onClick={() => removeSharedPath(i)}
                      className="shrink-0 rounded p-0.5 text-text-tertiary hover:text-text-primary transition"
                    >
                      <TrashIcon className="h-3.5 w-3.5" />
                    </button>
                  </div>
                );
              })}
            </div>
            <div className="flex items-center gap-3 mt-2">
              <button
                onClick={addSharedPath}
                className="flex items-center gap-1 text-sm text-accent hover:underline"
              >
                <PlusIcon className="h-4 w-4" />
                Add path
              </button>
              <button
                onClick={() => setBrowseOpen(true)}
                className="flex items-center gap-1 text-sm text-accent hover:underline"
              >
                <FolderOpenIcon className="h-4 w-4" />
                Browse
              </button>
            </div>
          </div>
        </SectionPanel>
      )}

      {sandboxEnabled && (
        <FileBrowserModal
          open={browseOpen}
          onClose={() => setBrowseOpen(false)}
          onSelect={handleBrowseSelect}
          allowFolders
          confirmLabel="Select"
        />
      )}
    </div>
  );
}
