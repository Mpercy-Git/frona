"use client";

import { useState, useEffect } from "react";
import type { SandboxConfig } from "@/lib/config-types";
import { api } from "@/lib/api-client";
import { Toggle, Field, SectionHeader, SectionPanel } from "@/components/settings/field";
import { ShieldCheckIcon } from "@heroicons/react/24/outline";

interface SystemInfo {
  cpus: number;
  total_memory_bytes: number;
}

interface SandboxSettingsSectionProps {
  sandbox: SandboxConfig;
  onChange: (sandbox: SandboxConfig) => void;
}

function formatBytes(bytes: number): string {
  const gb = bytes / 1_073_741_824;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / 1_048_576).toFixed(0)} MB`;
}

export function SandboxSettingsSection({ sandbox, onChange }: SandboxSettingsSectionProps) {
  const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);

  useEffect(() => {
    api.get<SystemInfo>("/api/system/info").then(setSysInfo).catch(() => {});
  }, []);

  return (
    <div>
      <SectionHeader title="Sandbox" description="Execution environment and resource limits" icon={ShieldCheckIcon} />
      <div className="flex items-start gap-3 rounded-lg border border-warning/30 bg-warning/5 p-4 mb-4">
        <ShieldCheckIcon className="h-5 w-5 text-warning shrink-0 mt-0.5" />
        <p className="text-sm text-text-secondary leading-relaxed">
          The sandbox isolates agent-executed commands by restricting filesystem access, network connections, and system resource usage.
          Keeping it enabled is strongly recommended.
        </p>
      </div>
      <SectionPanel>
        <Toggle
          label="Sandbox Enabled"
          description={sandbox.disabled
            ? "The sandbox is disabled — agents have unrestricted command execution"
            : "Restrict filesystem and network access for CLI tool execution"}
          value={!sandbox.disabled}
          onChange={(enabled) => onChange({ ...sandbox, disabled: !enabled })}
        />

        <Toggle
          label="Default Network Access"
          description={sandbox.default_network_access
            ? "Agents have outbound network access by default. Restrict with forbid policies."
            : "Agents have no network access unless explicitly granted by policy."}
          value={sandbox.default_network_access}
          onChange={(value) => onChange({ ...sandbox, default_network_access: value })}
        />

        <Field label="Default Timeout (seconds)" description="Default execution timeout for sandboxed tool calls. 0 means no timeout. Per-agent settings override this.">
          <input
            type="number"
            value={sandbox.timeout_secs}
            onChange={(e) => onChange({ ...sandbox, timeout_secs: Math.max(0, Number(e.target.value)) })}
            min={0}
            max={3600}
            placeholder="0"
            className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
          />
        </Field>

        <div className="grid grid-cols-2 gap-x-6">
          <Field label="Global CPU Limit (%)" description="Kill processes if total CPU across all agents exceeds this">
            <div className="flex items-center gap-3">
              <input
                type="number"
                value={sandbox.max_total_cpu_pct}
                onChange={(e) => onChange({ ...sandbox, max_total_cpu_pct: Number(e.target.value) })}
                min={1}
                max={100}
                placeholder="90"
                className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {sysInfo && (
                <span className="text-xs text-text-tertiary">
                  {parseFloat(((sandbox.max_total_cpu_pct / 100) * sysInfo.cpus).toFixed(1))} of {sysInfo.cpus} cores
                </span>
              )}
            </div>
          </Field>
          <Field label="Per-Agent CPU Limit (%)" description="Kill process if agent CPU exceeds this">
            <div className="flex items-center gap-3">
              <input
                type="number"
                value={sandbox.max_cpu_pct}
                onChange={(e) => onChange({ ...sandbox, max_cpu_pct: Number(e.target.value) })}
                min={1}
                max={100}
                placeholder="80"
                className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {sysInfo && (
                <span className="text-xs text-text-tertiary">
                  {parseFloat(((sandbox.max_cpu_pct / 100) * sysInfo.cpus).toFixed(1))} of {sysInfo.cpus} cores
                </span>
              )}
            </div>
          </Field>
        </div>

        <div className="grid grid-cols-2 gap-x-6">
          <Field label="Global Memory Limit (%)" description="Kill processes if total memory across all agents exceeds this">
            <div className="flex items-center gap-3">
              <input
                type="number"
                value={sandbox.max_total_memory_pct}
                onChange={(e) => onChange({ ...sandbox, max_total_memory_pct: Number(e.target.value) })}
                min={1}
                max={100}
                placeholder="90"
                className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {sysInfo && (
                <span className="text-xs text-text-tertiary">
                  {formatBytes((sandbox.max_total_memory_pct / 100) * sysInfo.total_memory_bytes)} of {formatBytes(sysInfo.total_memory_bytes)}
                </span>
              )}
            </div>
          </Field>
          <Field label="Per-Agent Memory Limit (%)" description="Kill process if agent memory exceeds this">
            <div className="flex items-center gap-3">
              <input
                type="number"
                value={sandbox.max_memory_pct}
                onChange={(e) => onChange({ ...sandbox, max_memory_pct: Number(e.target.value) })}
                min={1}
                max={100}
                placeholder="80"
                className="block w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {sysInfo && (
                <span className="text-xs text-text-tertiary">
                  {formatBytes((sandbox.max_memory_pct / 100) * sysInfo.total_memory_bytes)} of {formatBytes(sysInfo.total_memory_bytes)}
                </span>
              )}
            </div>
          </Field>
        </div>
      </SectionPanel>
    </div>
  );
}
