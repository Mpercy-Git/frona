"use client";

import { SwatchIcon } from "@heroicons/react/24/outline";
import { useTheme } from "@/lib/theme";
import { SectionHeader, SectionPanel } from "../field";

const themeModes = [
  { value: "system" as const, label: "System" },
  { value: "light" as const, label: "Light" },
  { value: "dark" as const, label: "Dark" },
];

export function ThemeSection() {
  const { mode, setMode } = useTheme();

  return (
    <div className="space-y-6">
      <SectionHeader title="Theme" description="Customize the appearance" icon={SwatchIcon} />

      <SectionPanel title="Appearance">
        <div className="flex gap-2">
          {themeModes.map(({ value, label }) => (
            <button
              key={value}
              onClick={() => setMode(value)}
              className={`flex-1 rounded-lg px-3 py-2 text-sm font-medium transition ${
                mode === value
                  ? "bg-accent text-surface"
                  : "bg-surface text-text-secondary hover:bg-surface-tertiary"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      </SectionPanel>
    </div>
  );
}
