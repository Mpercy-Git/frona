"use client";

import { useEffect, useMemo, useState } from "react";
import { ClockIcon, InformationCircleIcon } from "@heroicons/react/24/outline";
import type { ServerConfig } from "@/lib/config-types";
import { SectionHeader, SectionPanel } from "@/components/settings/field";
import { ComboboxInput } from "@/components/settings/combobox";
import { api } from "@/lib/api-client";

interface TimezoneSectionProps {
  server: ServerConfig;
  onChange: (server: ServerConfig) => void;
}

const TIMEZONE_AUTO_DETECT = "(auto-detect from server environment)";

export function TimezoneSection({ server, onChange }: TimezoneSectionProps) {
  const [timezones, setTimezones] = useState<string[]>([]);

  useEffect(() => {
    api.get<string[]>("/api/system/timezones").then(setTimezones).catch(() => {});
  }, []);

  const timezoneItems = useMemo(
    () => [
      { value: "", label: TIMEZONE_AUTO_DETECT },
      ...timezones.map((tz) => ({ value: tz, label: tz.replace(/_/g, " ") })),
    ],
    [timezones],
  );

  const browserTimezone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone;
    } catch {
      return null;
    }
  }, []);

  return (
    <div>
      <SectionHeader
        title="Timezone"
        description="The default timezone used for scheduling, reminders, and the time context shown to agents."
        icon={ClockIcon}
      />
      <div className="flex items-start gap-3 rounded-lg border border-accent/30 bg-accent/5 p-4 mb-4">
        <InformationCircleIcon className="h-5 w-5 text-accent shrink-0 mt-0.5" />
        <p className="text-sm text-text-secondary leading-relaxed">
          Server default. Each user can override it in their profile, and individual tasks can specify a different zone.
          Daylight saving transitions are handled automatically by the IANA database. Takes effect after restart.
        </p>
      </div>
      <SectionPanel>
        <ComboboxInput
          label="Default Timezone"
          description="Leave on auto-detect to read the TZ env var or /etc/localtime at startup, falling back to UTC."
          value={server.timezone}
          items={timezoneItems}
          onChange={(timezone) => onChange({ ...server, timezone })}
          placeholder={TIMEZONE_AUTO_DETECT}
          allowFreeText={false}
        />
        {browserTimezone && server.timezone !== browserTimezone && (
          <button
            type="button"
            onClick={() => onChange({ ...server, timezone: browserTimezone })}
            className="text-xs text-accent hover:underline"
          >
            Use browser timezone: {browserTimezone}
          </button>
        )}
      </SectionPanel>
    </div>
  );
}
