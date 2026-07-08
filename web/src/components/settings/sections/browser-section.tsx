"use client";

import type { BrowserConfig } from "@/lib/config-types";
import { TextInput, NumberInput, Toggle, SectionHeader, SectionPanel } from "@/components/settings/field";
import { GlobeAltIcon } from "@heroicons/react/24/outline";

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
            onChange={(ws_url) => onChange({ ...browser, ws_url })}
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
        </>
      )}
      </SectionPanel>
    </div>
  );
}
