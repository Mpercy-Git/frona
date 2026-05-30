"use client";

import type { VoiceConfig } from "@/lib/config-types";
import { isSensitiveSet } from "@/lib/config-types";
import { TextInput, SelectInput, SensitiveInput, Toggle, SectionHeader, SectionPanel } from "@/components/settings/field";
import { PhoneIcon } from "@heroicons/react/24/outline";

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
            label="Inbound Fallback User ID"
            description="User that owns calls matching the static inbound allowlist"
            value={voice.inbound_user_id}
            onChange={(inbound_user_id) => onChange({ ...voice, inbound_user_id })}
            placeholder="user-id"
          />

          <TextInput
            label="Inbound Agent ID"
            description="Agent that answers inbound calls (defaults to receptionist)"
            value={voice.inbound_agent_id}
            onChange={(inbound_agent_id) => onChange({ ...voice, inbound_agent_id })}
            placeholder="receptionist"
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
            placeholder="+15551234567, +447700900123"
          />

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
    </div>
  );
}
