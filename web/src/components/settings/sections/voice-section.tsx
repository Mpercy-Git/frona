"use client";

import type { VoiceConfig } from "@/lib/config-types";
import { isSensitiveSet } from "@/lib/config-types";
import { TextInput, SelectInput, SensitiveInput, SectionHeader, SectionPanel } from "@/components/settings/field";
import { PhoneIcon } from "@heroicons/react/24/outline";

interface VoiceSectionProps {
  voice: VoiceConfig;
  onChange: (voice: VoiceConfig) => void;
}

const voiceProviders = [
  { value: "twilio", label: "Twilio" },
];

function inferProvider(voice: VoiceConfig): string | null {
  if (voice.provider) return voice.provider;
  if (isSensitiveSet(voice.twilio_account_sid)) return "twilio";
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

        </>
      )}
      </SectionPanel>
    </div>
  );
}
