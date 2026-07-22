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

          <Toggle
            label="Silence Filling"
            description="Send periodic filler phrases while the agent is processing, but only when the other party is a registered user (a user calling in, or the agent calling a user). Calls with third parties narrate their own progress and are unaffected."
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
                description="Comma- or newline-separated phrases. Each interval advances to the next phrase in order. Leave empty for defaults."
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

      {voice.inbound_enabled && (
        <>
          <InboundSettings />
          <CallerAllowlist />
        </>
      )}
    </div>
  );
}

interface InboundSettingsData {
  agent: string | null;
  greeting: string | null;
}

function InboundSettings() {
  const [agent, setAgent] = useState("");
  const [greeting, setGreeting] = useState("");
  const [saved, setSaved] = useState<InboundSettingsData>({ agent: null, greeting: null });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const apply = (data: InboundSettingsData) => {
    setAgent(data.agent ?? "");
    setGreeting(data.greeting ?? "");
    setSaved(data);
  };

  const load = useCallback(async () => {
    try {
      apply(await api.get<InboundSettingsData>("/api/voice/inbound-settings"));
    } catch {
      setError("Failed to load inbound settings");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  const save = async () => {
    setError(null);
    try {
      apply(await api.put<InboundSettingsData>("/api/voice/inbound-settings", {
        agent: agent.trim(),
        greeting: greeting.trim(),
      }));
    } catch {
      setError("Failed to save inbound settings");
    }
  };

  const dirty =
    agent.trim() !== (saved.agent ?? "") || greeting.trim() !== (saved.greeting ?? "");

  return (
    <SectionPanel title="Inbound Answering">
      <p className="text-xs text-text-tertiary mb-3">
        Settings for how your inbound calls are answered.
      </p>
      {loading && <p className="text-sm text-text-tertiary">Loading...</p>}
      {error && <p className="text-sm text-error-text mb-2">{error}</p>}
      {!loading && (
        <div className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-text-tertiary mb-1">Answering Agent</label>
            <input
              type="text"
              value={agent}
              onChange={(e) => setAgent(e.target.value)}
              placeholder="receptionist (handle or name)"
              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
            />
            <p className="text-xs text-text-tertiary mt-1">
              One of your agents by handle or name. Leave empty to use your receptionist.
            </p>
          </div>
          <div>
            <label className="block text-xs font-medium text-text-tertiary mb-1">Welcome Greeting</label>
            <input
              type="text"
              value={greeting}
              onChange={(e) => setGreeting(e.target.value)}
              placeholder="Hi, thanks for calling..."
              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
            />
            <p className="text-xs text-text-tertiary mt-1">
              Spoken when a call connects. Leave empty to use the server default.
            </p>
          </div>
          <div className="flex justify-end">
            <button
              onClick={save}
              disabled={!dirty}
              className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
            >
              Save
            </button>
          </div>
        </div>
      )}
    </SectionPanel>
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
