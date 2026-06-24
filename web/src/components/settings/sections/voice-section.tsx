"use client";

import type { VoiceConfig } from "@/lib/config-types";
import { isSensitiveSet } from "@/lib/config-types";
import { TextInput, SelectInput, SensitiveInput, Toggle, SectionHeader, SectionPanel } from "@/components/settings/field";
import { PhoneIcon } from "@heroicons/react/24/outline";
import { useState, useEffect, useCallback } from "react";
import { api } from "@/lib/api-client";

interface AllowlistEntry {
  phone: string;
  name?: string;
}

interface VoiceSectionProps {
  voice: VoiceConfig;
  onChange: (voice: VoiceConfig) => void;
}

const voiceProviders = [
  { value: "twilio", label: "Twilio" },
  { value: "plivo", label: "Plivo" },
];

function inferProvider(voice: VoiceConfig): string | null {
  if (voice.provider) return voice.provider;
  if (isSensitiveSet(voice.twilio_account_sid)) return "twilio";
  if (isSensitiveSet(voice.plivo_auth_id)) return "plivo";
  return null;
}

function parseAllowlist(raw: string): string[] {
  return raw
    .split(/[,\n]/)
    .map((p) => p.trim())
    .filter(Boolean);
}

export function VoiceSection({ voice, onChange }: VoiceSectionProps) {
  const effectiveProvider = inferProvider(voice);

  return (
    <div>
      <SectionHeader title="Voice" description="Voice call provider for phone-based agent interactions" icon={PhoneIcon} />
      <SectionPanel>

      <SelectInput
        label="Provider"
        description="Select a voice provider"
        value={effectiveProvider}
        onChange={(provider) => onChange({ ...voice, provider })}
        options={voiceProviders}
      />

      {effectiveProvider === "twilio" && (
        <>
          <SensitiveInput
            label="Account SID"
            description="Twilio account identifier"
            value={voice.twilio_account_sid}
            onChange={(twilio_account_sid) => onChange({ ...voice, twilio_account_sid })}
            placeholder="ACxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
          />

          <SensitiveInput
            label="Auth Token"
            description="Twilio authentication token"
            value={voice.twilio_auth_token}
            onChange={(twilio_auth_token) => onChange({ ...voice, twilio_auth_token })}
            placeholder="Enter auth token"
          />

          <TextInput
            label="From Number"
            description="Twilio phone number to make calls from"
            value={voice.twilio_from_number}
            onChange={(twilio_from_number) => onChange({ ...voice, twilio_from_number })}
            placeholder="+15551234567"
          />

          <TextInput
            label="Voice ID"
            description="Voice identifier for text-to-speech"
            value={voice.twilio_voice_id}
            onChange={(twilio_voice_id) => onChange({ ...voice, twilio_voice_id })}
            placeholder="Polly.Amy"
          />

          <TextInput
            label="Speech Model"
            description="Speech recognition model"
            value={voice.twilio_speech_model}
            onChange={(twilio_speech_model) => onChange({ ...voice, twilio_speech_model })}
            placeholder="phone_call"
          />

          <TextInput
            label="TTS Provider"
            description="Text-to-speech provider (e.g. elevenlabs, polly). Leave empty for default (Polly)."
            value={voice.twilio_tts_provider}
            onChange={(twilio_tts_provider) => onChange({ ...voice, twilio_tts_provider })}
            placeholder="elevenlabs"
          />

          <TextInput
            label="Callback Base URL"
            description="Public URL Twilio should use for voice webhooks (defaults to server.base_url)"
            value={voice.callback_base_url}
            onChange={(callback_base_url) => onChange({ ...voice, callback_base_url })}
            placeholder="https://your-public-domain.com"
          />

          <Toggle
            label="Enable Inbound Call Answering"
            description="Allow Twilio inbound calls at /api/voice/twilio/inbound"
            value={voice.inbound_enabled}
            onChange={(inbound_enabled) => onChange({ ...voice, inbound_enabled })}
          />

          <TextInput
            label="Inbound Fallback User"
            description="User ID or username that owns calls matching the static inbound allowlist"
            value={voice.inbound_user_id}
            onChange={(inbound_user_id) => onChange({ ...voice, inbound_user_id })}
            placeholder="user-id or username"
          />

          <TextInput
            label="Inbound Agent"
            description="Agent ID, handle, or name that answers inbound calls (defaults to receptionist)"
            value={voice.inbound_agent_id}
            onChange={(inbound_agent_id) => onChange({ ...voice, inbound_agent_id })}
            placeholder="receptionist (handle or name)"
          />

          <TextInput
            label="Inbound Welcome Greeting"
            description="Greeting spoken when an inbound call connects"
            value={voice.inbound_welcome_greeting}
            onChange={(inbound_welcome_greeting) => onChange({ ...voice, inbound_welcome_greeting })}
            placeholder="Hi, thanks for calling..."
          />

          <TextInput
            label="Inbound Static Allowlist"
            description="Comma- or newline-separated E.164 phone numbers allowed for inbound answering"
            value={voice.inbound_allowlist?.join(", ") ?? ""}
            onChange={(raw) => onChange({ ...voice, inbound_allowlist: parseAllowlist(raw) })}
            placeholder="+155****4567, +447****0123"
          />

          <Toggle
            label="Silence Filling"
            description="Send periodic filler phrases to the caller while the agent is processing"
            value={voice.silence_fill_enabled}
            onChange={(silence_fill_enabled) => onChange({ ...voice, silence_fill_enabled })}
          />

          {voice.silence_fill_enabled && (
            <>
              <TextInput
                label="Initial Silence Delay (seconds)"
                description="Seconds of silence before the first filler phrase is sent"
                value={String(voice.silence_fill_initial_delay_secs ?? 3)}
                onChange={(raw) => {
                  const n = parseInt(raw, 10);
                  onChange({ ...voice, silence_fill_initial_delay_secs: isNaN(n) ? 3 : Math.max(1, n) });
                }}
                placeholder="3"
              />
              <TextInput
                label="Filler Interval (seconds)"
                description="Seconds between successive filler phrases"
                value={String(voice.silence_fill_interval_secs ?? 7)}
                onChange={(raw) => {
                  const n = parseInt(raw, 10);
                  onChange({ ...voice, silence_fill_interval_secs: isNaN(n) ? 7 : Math.max(1, n) });
                }}
                placeholder="7"
              />
              <TextInput
                label="Filler Phrases"
                description="Comma- or newline-separated phrases. One is chosen at random each interval. Leave empty for defaults."
                value={voice.silence_fill_phrases?.join(", ") ?? ""}
                onChange={(raw) => {
                  const phrases = raw.split(/[,\n]/).map(s => s.trim()).filter(s => s.length > 0);
                  onChange({ ...voice, silence_fill_phrases: phrases });
                }}
                placeholder="Just a moment..., Let me look into that..., One moment please..."
              />
            </>
          )}

        </>
      )}

      {effectiveProvider === "plivo" && (
        <>
          <SensitiveInput
            label="Auth ID"
            description="Plivo authentication ID"
            value={voice.plivo_auth_id}
            onChange={(plivo_auth_id) => onChange({ ...voice, plivo_auth_id })}
            placeholder="MAxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
          />

          <SensitiveInput
            label="Auth Token"
            description="Plivo authentication token"
            value={voice.plivo_auth_token}
            onChange={(plivo_auth_token) => onChange({ ...voice, plivo_auth_token })}
            placeholder="Enter auth token"
          />

          <TextInput
            label="From Number"
            description="Plivo phone number to make calls from"
            value={voice.plivo_from_number}
            onChange={(plivo_from_number) => onChange({ ...voice, plivo_from_number })}
            placeholder="+15551234567"
          />

          <TextInput
            label="Callback Base URL"
            description="Public URL Plivo should use for voice webhooks (defaults to server.base_url)"
            value={voice.callback_base_url}
            onChange={(callback_base_url) => onChange({ ...voice, callback_base_url })}
            placeholder="https://your-public-domain.com"
          />
        </>
      )}
      </SectionPanel>

      {voice.inbound_enabled && <CallerAllowlist />}
    </div>
  );
}

function CallerAllowlist() {
  const [entries, setEntries] = useState<AllowlistEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [newPhone, setNewPhone] = useState("");
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await api.get<AllowlistEntry[]>("/api/voice/allowlist");
      setEntries(data);
    } catch {
      setError("Failed to load allowlist");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  const addEntry = async () => {
    if (!newPhone.trim()) return;
    setError(null);
    try {
      const data = await api.post<AllowlistEntry[]>("/api/voice/allowlist", {
        phone: newPhone.trim(),
        name: newName.trim() || null,
      });
      setEntries(data);
      setNewPhone("");
      setNewName("");
    } catch {
      setError("Failed to add entry");
    }
  };

  const removeEntry = async (phone: string) => {
    setError(null);
    try {
      const encoded = encodeURIComponent(phone);
      const data = await api.delete<AllowlistEntry[]>(`/api/voice/allowlist/${encoded}`);
      setEntries(data);
    } catch {
      setError("Failed to remove entry");
    }
  };

  return (
    <SectionPanel title="Caller Allowlist">
      <p className="text-xs text-text-tertiary mb-3">
        Phone numbers allowed for inbound answering, with optional names. When a named
        caller dials in, the agent is told who is calling.
      </p>
      {loading && <p className="text-sm text-text-tertiary">Loading...</p>}
      {error && <p className="text-sm text-error-text mb-2">{error}</p>}
      {entries.length > 0 && (
        <div className="space-y-1 mb-4">
          {entries.map((entry) => (
            <div key={entry.phone} className="flex items-center justify-between rounded-lg border border-border bg-surface px-3 py-2">
              <div className="flex items-center gap-3">
                <span className="text-sm text-text-primary font-mono">{entry.phone}</span>
                {entry.name && (
                  <span className="text-sm text-text-secondary">— {entry.name}</span>
                )}
              </div>
              <button
                onClick={() => removeEntry(entry.phone)}
                className="text-xs text-text-tertiary hover:text-error-text"
              >
                Remove
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="flex items-end gap-2">
        <div className="flex-1">
          <label className="block text-xs font-medium text-text-tertiary mb-1">Phone</label>
          <input
            type="tel"
            value={newPhone}
            onChange={(e) => setNewPhone(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") addEntry(); }}
            placeholder="+44 7xxx xxx xxx"
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
          />
        </div>
        <div className="flex-1">
          <label className="block text-xs font-medium text-text-tertiary mb-1">Name (optional)</label>
          <input
            type="text"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") addEntry(); }}
            placeholder="e.g. Mum, Dr. Smith"
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
          />
        </div>
        <button
          onClick={addEntry}
          disabled={!newPhone.trim()}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
        >
          Add
        </button>
      </div>
    </SectionPanel>
  );
}
