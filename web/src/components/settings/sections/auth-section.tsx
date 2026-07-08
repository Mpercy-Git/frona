"use client";

import type { AuthConfig } from "@/lib/config-types";
import { NumberInput, SensitiveInput, SectionHeader, SectionPanel, Toggle } from "@/components/settings/field";
import { KeyIcon } from "@heroicons/react/24/outline";

interface AuthSectionProps {
  auth: AuthConfig;
  onChange: (auth: AuthConfig) => void;
}

function generateSecret(): string {
  return crypto.randomUUID() + crypto.randomUUID();
}

export function AuthSection({ auth, onChange }: AuthSectionProps) {
  return (
    <div>
      <SectionHeader title="Authentication" description="Token secrets and session expiry settings" icon={KeyIcon} />
      <SectionPanel>

      <SensitiveInput
        label="Encryption Secret"
        description="Secret key used for encrypting tokens and sessions"
        value={auth.encryption_secret}
        onChange={(encryption_secret) => onChange({ ...auth, encryption_secret })}
        placeholder="Enter encryption secret"
        onGenerate={generateSecret}
      />

      <NumberInput
        label="Access Token Expiry (minutes)"
        description="How long access tokens remain valid"
        value={Math.round(auth.access_token_expiry_secs / 60)}
        onChange={(mins) => onChange({ ...auth, access_token_expiry_secs: mins * 60 })}
        min={1}
        placeholder="15"
      />

      <NumberInput
        label="Refresh Token Expiry (days)"
        description="How long refresh tokens remain valid"
        value={Math.round(auth.refresh_token_expiry_secs / 86400)}
        onChange={(days) => onChange({ ...auth, refresh_token_expiry_secs: days * 86400 })}
        min={1}
        placeholder="7"
      />

      <NumberInput
        label="Presign Expiry (hours)"
        description="How long presigned URLs remain valid"
        value={Math.round(auth.presign_expiry_secs / 3600)}
        onChange={(hours) => onChange({ ...auth, presign_expiry_secs: hours * 3600 })}
        min={1}
        placeholder="24"
      />

      <Toggle
        label="Allow open registration"
        description="When off, the public registration form is disabled. Admins can still add users from Settings → Users."
        value={auth.allow_registration}
        onChange={(allow_registration) => onChange({ ...auth, allow_registration })}
      />
      </SectionPanel>
    </div>
  );
}
