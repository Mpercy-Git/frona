"use client";

import { useState, useEffect, useMemo } from "react";
import { UserCircleIcon } from "@heroicons/react/24/outline";
import { useAuth } from "@/lib/auth";
import { api } from "@/lib/api-client";
import { SectionHeader, SectionPanel } from "../field";
import { ComboboxInput } from "../combobox";

interface SystemInfo {
  server_timezone: string;
}

export function ProfileSection() {
  const { user, revalidate } = useAuth();
  const [timezone, setTimezone] = useState(user?.timezone ?? "");
  const [phone, setPhone] = useState(user?.phone ?? "");
  const [email, setEmail] = useState(user?.email ?? "");
  const [name, setName] = useState(user?.name ?? "");
  const [saving, setSaving] = useState(false);
  const [savingPhone, setSavingPhone] = useState(false);
  const [savingEmail, setSavingEmail] = useState(false);
  const [savingName, setSavingName] = useState(false);
  const [profileError, setProfileError] = useState<string | null>(null);
  const [timezones, setTimezones] = useState<string[]>([]);
  const [serverTimezone, setServerTimezone] = useState<string>("");

  useEffect(() => {
    api.get<string[]>("/api/system/timezones").then(setTimezones).catch(() => {});
    api.get<SystemInfo>("/api/system/info").then((i) => setServerTimezone(i.server_timezone ?? "")).catch(() => {});
  }, []);

  const effectiveTimezone = timezone || serverTimezone;
  const usingServerDefault = !timezone && !!serverTimezone;

  const timezoneItems = useMemo(
    () => timezones.map((tz) => ({ value: tz, label: tz.replace(/_/g, " ") })),
    [timezones],
  );

  const detectedTimezone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone;
    } catch {
      return null;
    }
  }, []);

  // Always send the full profile so a partial save can't clobber other fields
  // (the backend treats a missing Option field as "clear it").
  const saveProfile = async (patch: {
    timezone?: string | null;
    phone?: string | null;
    email?: string;
    name?: string;
  }) => {
    setProfileError(null);
    await api.put("/api/auth/profile", {
      timezone: timezone || null,
      phone: phone || null,
      email,
      name,
      ...patch,
    });
    await revalidate();
  };

  const saveTimezone = async (tz: string) => {
    setTimezone(tz);
    if (!timezones.includes(tz)) return;
    setSaving(true);
    try {
      await saveProfile({ timezone: tz || null });
    } finally {
      setSaving(false);
    }
  };

  const savePhone = async () => {
    if (phone === (user?.phone ?? "")) return;
    setSavingPhone(true);
    try {
      await saveProfile({ phone: phone || null });
    } finally {
      setSavingPhone(false);
    }
  };

  const saveEmail = async () => {
    const trimmed = email.trim();
    if (trimmed === (user?.email ?? "")) return;
    if (!trimmed) {
      setEmail(user?.email ?? "");
      return;
    }
    setSavingEmail(true);
    try {
      await saveProfile({ email: trimmed });
    } catch (e) {
      setProfileError(e instanceof Error ? e.message : "Failed to update email");
      setEmail(user?.email ?? "");
    } finally {
      setSavingEmail(false);
    }
  };

  const saveName = async () => {
    const trimmed = name.trim();
    if (trimmed === (user?.name ?? "")) return;
    if (!trimmed) {
      setName(user?.name ?? "");
      return;
    }
    setSavingName(true);
    try {
      await saveProfile({ name: trimmed });
    } catch (e) {
      setProfileError(e instanceof Error ? e.message : "Failed to update name");
      setName(user?.name ?? "");
    } finally {
      setSavingName(false);
    }
  };

  return (
    <div className="space-y-6">
      <SectionHeader title="Profile" description="Your account information" icon={UserCircleIcon} />

      {user && (
        <SectionPanel title="Account">
          <div className="space-y-3">
            <div>
              <label className="block text-xs font-medium text-text-tertiary mb-1">Name</label>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                onBlur={saveName}
                onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
                placeholder="Your name"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {savingName && <p className="text-xs text-text-tertiary mt-1">Saving...</p>}
            </div>
            <div>
              <label className="block text-xs font-medium text-text-tertiary mb-1">Username</label>
              <p className="text-sm text-text-primary">@{user.handle}</p>
            </div>
            <div>
              <label className="block text-xs font-medium text-text-tertiary mb-1">Email</label>
              <input
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                onBlur={saveEmail}
                onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
                placeholder="you@example.com"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {savingEmail && <p className="text-xs text-text-tertiary mt-1">Saving...</p>}
            </div>
            {profileError && (
              <p className="text-xs text-error-text">{profileError}</p>
            )}
            <div>
              <label className="block text-xs font-medium text-text-tertiary mb-1">Phone</label>
              <input
                type="tel"
                value={phone}
                onChange={(e) => setPhone(e.target.value)}
                onBlur={savePhone}
                onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
                placeholder="+44 7xxx xxx xxx"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
              {savingPhone && (
                <p className="text-xs text-text-tertiary mt-1">Saving...</p>
              )}
            </div>
          </div>
        </SectionPanel>
      )}

      {user && (
        <SectionPanel title="Preferences">
          <div className="space-y-1">
            <ComboboxInput
              label="Timezone"
              value={timezone}
              items={timezoneItems}
              onChange={saveTimezone}
              placeholder="Select timezone..."
              allowFreeText={false}
            />
            {effectiveTimezone && (
              <p className="text-xs text-text-tertiary">
                {usingServerDefault
                  ? `Currently using server default: ${effectiveTimezone}`
                  : `Currently using: ${effectiveTimezone}`}
              </p>
            )}
            {detectedTimezone && timezone !== detectedTimezone && (
              <button
                type="button"
                onClick={() => saveTimezone(detectedTimezone)}
                className="text-xs text-accent hover:underline"
              >
                Use detected: {detectedTimezone}
              </button>
            )}
            {saving && (
              <p className="text-xs text-text-tertiary">Saving...</p>
            )}
          </div>
        </SectionPanel>
      )}
    </div>
  );
}
