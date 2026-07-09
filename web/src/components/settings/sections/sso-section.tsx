"use client";

import type { SsoConfig } from "@/lib/config-types";
import { TextInput, NumberInput, Toggle, SensitiveInput, SectionHeader, SectionPanel } from "@/components/settings/field";
import { ExclamationTriangleIcon, FingerPrintIcon } from "@heroicons/react/24/outline";

interface SsoSectionProps {
  sso: SsoConfig;
  onChange: (sso: SsoConfig) => void;
  hasBaseUrl?: boolean;
}

export function SsoSection({ sso, onChange, hasBaseUrl }: SsoSectionProps) {
  return (
    <div>
      <SectionHeader title="Single Sign-On" description="OpenID Connect SSO configuration" icon={FingerPrintIcon} />
      {sso.enabled && hasBaseUrl === false && (
        <div className="flex items-start gap-3 rounded-lg border border-warning/30 bg-warning/5 p-4 mb-4">
          <ExclamationTriangleIcon className="h-5 w-5 text-warning shrink-0 mt-0.5" />
          <p className="text-sm text-text-secondary leading-relaxed">
            SSO requires a public Base URL to construct the OAuth redirect URI.
            Set it in the Server section.
          </p>
        </div>
      )}
      <SectionPanel>

      <Toggle
        label="Enabled"
        description="Enable SSO authentication"
        value={sso.enabled}
        onChange={(enabled) => onChange({ ...sso, enabled })}
      />

      {sso.enabled && (
        <>
          <TextInput
            label="Authority URL"
            description="OpenID Connect discovery endpoint"
            value={sso.authority}
            onChange={(authority) => onChange({ ...sso, authority })}
            placeholder="https://auth.example.com"
          />

          <TextInput
            label="Client ID"
            description="OAuth client identifier"
            value={sso.client_id}
            onChange={(client_id) => onChange({ ...sso, client_id })}
            placeholder="your-client-id"
          />

          <SensitiveInput
            label="Client Secret"
            description="OAuth client secret"
            value={sso.client_secret}
            onChange={(client_secret) => onChange({ ...sso, client_secret })}
            placeholder="Enter client secret"
          />

          <TextInput
            label="Scopes"
            description="Space-separated list of OAuth scopes to request"
            value={sso.scopes}
            onChange={(scopes) => onChange({ ...sso, scopes })}
            placeholder="openid email"
          />

          <Toggle
            label="SSO Only"
            description="Only allow SSO authentication, disabling native login"
            value={sso.disable_local_auth}
            onChange={(disable_local_auth) => onChange({ ...sso, disable_local_auth })}
            warning="Native email/password authentication will be disabled"
          />

          <Toggle
            label="Signups Match Email"
            description="Link SSO accounts to existing users by email address"
            value={sso.signups_match_email}
            onChange={(signups_match_email) => onChange({ ...sso, signups_match_email })}
          />

          <Toggle
            label="Allow Unknown Email Verification"
            description="Trust email verification claims from the SSO provider"
            value={sso.allow_unknown_email_verification}
            onChange={(allow_unknown_email_verification) => onChange({ ...sso, allow_unknown_email_verification })}
          />

          <NumberInput
            label="Client Cache Expiration (seconds)"
            description="How long to cache the OIDC client configuration"
            value={sso.client_cache_expiration}
            onChange={(client_cache_expiration) => onChange({ ...sso, client_cache_expiration })}
            min={0}
          />
        </>
      )}
      </SectionPanel>
    </div>
  );
}
