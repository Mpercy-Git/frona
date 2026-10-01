"use client";

import { TextInput, SectionHeader, SectionPanel } from "@/components/settings/field";
import { MicrophoneIcon } from "@heroicons/react/24/outline";

interface VoiceSectionProps {
  voiceId: string;
  onVoiceIdChange: (voiceId: string) => void;
}

export function VoiceSection({ voiceId, onVoiceIdChange }: VoiceSectionProps) {
  return (
    <div>
      <SectionHeader title="Voice" description="Text-to-speech voice for live phone calls" icon={MicrophoneIcon} />
      <SectionPanel>
        <p className="text-sm text-text-tertiary">
          The voice this agent speaks with on a live phone call — including when a call is transferred to it mid-conversation.
          Leave blank to use the server&apos;s default voice.
        </p>
        <TextInput
          label="Voice ID"
          value={voiceId}
          onChange={onVoiceIdChange}
          placeholder="Server default"
        />
      </SectionPanel>
    </div>
  );
}
