"use client";

import type { MailConfig, SmtpTls } from "@/lib/config-types";
import {
  TextInput,
  NumberInput,
  SelectInput,
  SensitiveInput,
  SectionHeader,
  SectionPanel,
} from "@/components/settings/field";
import { EnvelopeIcon, ExclamationTriangleIcon } from "@heroicons/react/24/outline";

interface MailSectionProps {
  mail: MailConfig;
  onChange: (mail: MailConfig) => void;
  /** Whether server.frontend_url or server.base_url is set. */
  hasFrontendUrl?: boolean;
}

const TLS_OPTIONS: { value: SmtpTls; label: string }[] = [
  { value: "starttls", label: "STARTTLS (usually port 587)" },
  { value: "implicit", label: "Implicit TLS (usually port 465)" },
  { value: "none", label: "None — plaintext" },
];

export function MailSection({ mail, onChange, hasFrontendUrl }: MailSectionProps) {
  const configured = mail.smtp_host.trim() !== "";

  return (
    <div>
      <SectionHeader
        title="Email"
        description="Outbound SMTP. Required for password reset — without it, users who forget their password can only be recovered by an admin."
        icon={EnvelopeIcon}
      />

      {configured && hasFrontendUrl === false && (
        <div className="flex items-start gap-3 rounded-lg border border-warning/30 bg-warning/5 p-4 mb-4">
          <ExclamationTriangleIcon className="h-5 w-5 text-warning shrink-0 mt-0.5" />
          <p className="text-sm text-text-secondary leading-relaxed">
            Reset links are built from the public frontend URL, which isn&apos;t set — every
            emailed link would be missing its host and fail to open. Set Base URL or
            Frontend URL in the Server section.
          </p>
        </div>
      )}

      <SectionPanel>
        <TextInput
          label="SMTP Host"
          description="Leave empty to disable outbound email entirely."
          value={mail.smtp_host}
          onChange={(smtp_host) => onChange({ ...mail, smtp_host })}
          placeholder="smtp.example.com"
        />

        {configured && (
          <>
            <NumberInput
              label="SMTP Port"
              description="587 for STARTTLS, 465 for implicit TLS, 25 or 1025 for plaintext relays."
              value={mail.smtp_port}
              onChange={(smtp_port) => onChange({ ...mail, smtp_port })}
              min={1}
              max={65535}
              placeholder="587"
            />

            <SelectInput
              label="Transport Security"
              description="How the connection is encrypted. Use plaintext only for a relay on localhost or a local mail catcher."
              value={mail.tls}
              onChange={(tls) => onChange({ ...mail, tls: (tls as SmtpTls) ?? "starttls" })}
              options={TLS_OPTIONS}
              allowEmpty={false}
            />

            <TextInput
              label="SMTP Username"
              description="Leave empty for an unauthenticated relay."
              value={mail.smtp_username ?? ""}
              onChange={(smtp_username) =>
                onChange({ ...mail, smtp_username: smtp_username || null })
              }
              placeholder="apikey"
            />

            <SensitiveInput
              label="SMTP Password"
              description="Prefer an app password or a scoped API key you can rotate."
              value={mail.smtp_password}
              onChange={(smtp_password) => onChange({ ...mail, smtp_password })}
              placeholder="Enter SMTP password"
            />

            <TextInput
              label="From Address"
              description="Envelope sender for outbound mail. Must be an address the relay is allowed to send as."
              value={mail.from_address}
              onChange={(from_address) => onChange({ ...mail, from_address })}
              placeholder="noreply@example.com"
            />

            <TextInput
              label="From Name"
              description="Display name shown alongside the sender address."
              value={mail.from_name}
              onChange={(from_name) => onChange({ ...mail, from_name })}
              placeholder="Frona"
            />
          </>
        )}
      </SectionPanel>
    </div>
  );
}
