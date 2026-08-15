"use client";

import { useState } from "react";
import type { BrowserConfig } from "@/lib/config-types";
import { TextInput, NumberInput, Toggle, SectionHeader, SectionPanel } from "@/components/settings/field";
import { GlobeAltIcon } from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import type { TestStatus } from "@/components/settings/sections/providers-section";
import { TestStatusIcon } from "@/components/settings/sections/providers-section";

interface BrowserSectionProps {
  browser: BrowserConfig | null;
  onChange: (browser: BrowserConfig | null) => void;
}

const defaultBrowser: BrowserConfig = {
  ws_url: "ws://browserless:3333",
  profiles_path: "/profiles",
  connection_timeout_ms: 30000,
};

export function BrowserSection({ browser, onChange }: BrowserSectionProps) {
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testError, setTestError] = useState<string | null>(null);

  const runTest = async () => {
    if (!browser?.ws_url) return;
    setTestStatus("testing");
    setTestError(null);
    try {
      await api.post("/api/browser/test", { ws_url: browser.ws_url, profiles_path: browser.profiles_path });
      setTestStatus("success");
    } catch (e) {
      setTestStatus("error");
      setTestError(e instanceof Error ? e.message : "Failed to connect");
    }
  };

  return (
    <div>
      <SectionHeader title="Browser Automation" description="Browserless connection for web automation tools" icon={GlobeAltIcon} />
      <SectionPanel>

      <Toggle
        label="Enabled"
        description="Enable browser automation capabilities"
        value={browser !== null}
        onChange={(enabled) => onChange(enabled ? { ...defaultBrowser } : null)}
      />

      {browser && (
        <>
          <TextInput
            label="WebSocket URL"
            description="Browserless WebSocket endpoint"
            value={browser.ws_url}
            onChange={(ws_url) => {
              onChange({ ...browser, ws_url });
              setTestStatus("idle");
              setTestError(null);
            }}
            placeholder="ws://browserless:3333"
          />

          <TextInput
            label="Profiles Path"
            description="Directory for storing browser profiles"
            value={browser.profiles_path}
            onChange={(profiles_path) => onChange({ ...browser, profiles_path })}
            placeholder="/profiles"
          />

          <NumberInput
            label="Connection Timeout (seconds)"
            description="Timeout for establishing browser connections"
            value={Math.round(browser.connection_timeout_ms / 1000)}
            onChange={(secs) => onChange({ ...browser, connection_timeout_ms: secs * 1000 })}
            min={1}
            placeholder="30"
          />

          <div className="flex items-center gap-3 pt-1">
            <button
              type="button"
              onClick={runTest}
              disabled={!browser.ws_url || testStatus === "testing"}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-text-secondary hover:bg-surface-tertiary disabled:opacity-50 transition"
            >
              {testStatus === "testing" ? "Testing…" : "Test connection"}
            </button>
            {testStatus !== "idle" && (
              <div className="flex items-center gap-1.5 text-xs text-text-tertiary">
                <TestStatusIcon status={testStatus} />
                {testStatus === "success" && "Connected"}
                {testStatus === "error" && <span className="text-error-text">{testError ?? "Could not connect"}</span>}
              </div>
            )}
          </div>
        </>
      )}
      </SectionPanel>
    </div>
  );
}
