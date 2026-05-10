"use client";

import { useState, useRef, useEffect, useCallback } from "react";
import { TextInput, Toggle, SectionHeader, SectionPanel, Field } from "@/components/settings/field";
import { UserCircleIcon } from "@heroicons/react/24/outline";
import { Logo } from "@/components/logo";
import { api } from "@/lib/api-client";
import data from "@emoji-mart/data";
import { Picker } from "emoji-mart";

const IDENTITY_FIELDS = [
  { key: "name", label: "Name", placeholder: "Pick something you like" },
  { key: "creature", label: "Creature", placeholder: "AI assistant, robot, familiar" },
  { key: "vibe", label: "Vibe", placeholder: "sharp, warm, chaotic, calm" },
  { key: "personality", label: "Personality", placeholder: "funny, blunt, poetic, sarcastic" },
  { key: "style", label: "Style", placeholder: "casual, formal, terse, elaborate" },
];

interface ProfileSectionProps {
  agentId: string;
  description: string;
  enabled: boolean;
  identity: Record<string, string>;
  onChange: (fields: { description?: string; enabled?: boolean }) => void;
  onIdentityChange: (identity: Record<string, string>) => void;
}

export function ProfileSection({ agentId, description, enabled, identity, onChange, onIdentityChange }: ProfileSectionProps) {
  const updateIdentityField = (key: string, value: string) => {
    onIdentityChange({ ...identity, [key]: value });
  };

  return (
    <div>
      <SectionHeader title="Profile" description="Every good robot deserves a backstory" icon={UserCircleIcon} />
      <SectionPanel>
        <div className="flex items-start justify-between gap-4">
          <AvatarField
            agentId={agentId}
            value={identity.avatar ?? ""}
            onChange={(v) => updateIdentityField("avatar", v)}
          />
          <Toggle label="Enabled" value={enabled} onChange={(v) => onChange({ enabled: v })} />
        </div>
        {IDENTITY_FIELDS.slice(0, 1).map((field) => (
          <TextInput
            key={field.key}
            label={field.label}
            value={identity[field.key] ?? ""}
            onChange={(v) => updateIdentityField(field.key, v)}
            placeholder={field.placeholder}
          />
        ))}
        <div className="space-y-1">
          <label className="inline-flex items-center gap-1 text-sm font-medium text-text-secondary">Specialty</label>
          <textarea
            value={description}
            onChange={(e) => onChange({ description: e.target.value })}
            rows={3}
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none resize-y"
          />
        </div>
        {IDENTITY_FIELDS.slice(1).map((field) => (
          <TextInput
            key={field.key}
            label={field.label}
            value={identity[field.key] ?? ""}
            onChange={(v) => updateIdentityField(field.key, v)}
            placeholder={field.placeholder}
          />
        ))}
        <EmojiField
          value={identity.emoji ?? ""}
          onChange={(v) => updateIdentityField("emoji", v)}
        />
      </SectionPanel>
    </div>
  );
}

function AvatarField({ agentId, value, onChange }: { agentId: string; value: string; onChange: (v: string) => void }) {
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFileChange = useCallback(async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const result = await api.uploadFile(`/api/agents/${agentId}/avatar`, file);
      onChange(result.url);
    } catch {
      const reader = new FileReader();
      reader.onload = () => {
        if (typeof reader.result === "string") {
          onChange(reader.result);
        }
      };
      reader.readAsDataURL(file);
    }
  }, [agentId, onChange]);

  const isImage = value && (value.startsWith("data:") || value.startsWith("http") || value.startsWith("/api/"));

  return (
    <div className="flex items-center gap-3">
      <button
        type="button"
        onClick={() => fileInputRef.current?.click()}
        className="relative shrink-0 group cursor-pointer"
      >
        {isImage ? (
          <img
            src={value}
            alt="Avatar"
            className="h-20 w-20 rounded-full object-cover border border-border group-hover:opacity-80 transition"
          />
        ) : (
          <div className="h-20 w-20 rounded-full border border-dashed border-border bg-surface flex items-center justify-center group-hover:border-accent transition">
            <Logo size={48} headOnly />
          </div>
        )}
      </button>
      {value && (
        <button
          type="button"
          onClick={() => onChange("")}
          className="text-xs text-text-tertiary hover:text-text-primary"
        >
          Remove
        </button>
      )}
      <input
        ref={fileInputRef}
        type="file"
        accept="image/*"
        onChange={handleFileChange}
        className="hidden"
      />
    </div>
  );
}

function EmojiField({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const pickerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  useEffect(() => {
    if (!open || !pickerRef.current) return;
    pickerRef.current.innerHTML = "";
    const picker = new Picker({
      data,
      theme: "dark",
      onEmojiSelect: (emoji: { native: string }) => {
        onChange(emoji.native);
        setOpen(false);
      },
    });
    pickerRef.current.appendChild(picker as unknown as Node);
  }, [open, onChange]);

  return (
    <Field label="Emoji">
      <div className="relative" ref={containerRef}>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setOpen(!open)}
            className="flex items-center gap-2 rounded-lg border border-border bg-surface px-3 py-2 text-sm hover:border-accent transition"
          >
            {value ? (
              <span className="text-2xl leading-none">{value}</span>
            ) : (
              <span className="text-text-tertiary">Pick an emoji</span>
            )}
          </button>
          {value && (
            <button
              type="button"
              onClick={() => onChange("")}
              className="text-xs text-text-tertiary hover:text-text-primary"
            >
              Clear
            </button>
          )}
        </div>
        {open && (
          <div className="absolute z-50 mt-2" ref={pickerRef} />
        )}
      </div>
    </Field>
  );
}
