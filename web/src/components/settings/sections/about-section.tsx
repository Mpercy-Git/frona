"use client";

import { useState, useEffect } from "react";
import { InformationCircleIcon } from "@heroicons/react/24/outline";
import { API_URL, getAccessToken } from "@/lib/api-client";
import { SectionHeader, SectionPanel } from "../field";

interface SystemInfo {
  version: string;
  cpus: number;
  total_memory_bytes: number;
  sandbox_driver: string;
}

function formatBytes(bytes: number): string {
  const gb = bytes / 1_073_741_824;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / 1_048_576).toFixed(0)} MB`;
}

const SANDBOX_LABELS: Record<string, string> = {
  macos: "MacOS",
  syd: "Syd",
  landlock: "Landlock",
  disabled: "Disabled",
};

export function AboutSection() {
  const [info, setInfo] = useState<SystemInfo | null>(null);

  useEffect(() => {
    async function fetchInfo() {
      try {
        const res = await fetch(`${API_URL}/api/system/info`, {
          headers: { Authorization: `Bearer ${getAccessToken()}` },
          credentials: "include",
        });
        if (res.ok) {
          setInfo(await res.json());
        }
      } catch {}
    }
    fetchInfo();
  }, []);

  const row = (label: string, value: string) => (
    <div>
      <label className="block text-xs font-medium text-text-tertiary mb-1">{label}</label>
      <p className="text-sm text-text-primary">{value}</p>
    </div>
  );

  return (
    <div className="space-y-6">
      <SectionHeader title="About" description="System information" icon={InformationCircleIcon} />

      <SectionPanel title="System">
        <div className="grid grid-cols-2 gap-x-6 gap-y-4">
          {row("Version", info?.version ?? "...")}
          {row("CPUs", info ? `${info.cpus} cores` : "...")}
          {row("Memory", info ? formatBytes(info.total_memory_bytes) : "...")}
          {row("Sandbox", info ? (SANDBOX_LABELS[info.sandbox_driver] ?? info.sandbox_driver) : "...")}
        </div>
      </SectionPanel>
    </div>
  );
}
