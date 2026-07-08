"use client";

import { useState, useRef, useEffect } from "react";
import { useHashNavigate } from "@/lib/hash-navigate";
import {
  AdjustmentsVerticalIcon,
  BeakerIcon,
  Cog6ToothIcon,
  CloudIcon,
  CubeIcon,
  GlobeAltIcon,
  KeyIcon,
  MagnifyingGlassIcon,
  MicrophoneIcon,
  PuzzlePieceIcon,
  UsersIcon,
  XMarkIcon,
} from "@heroicons/react/24/outline";
import { useAuth } from "@/lib/auth";
import { useMobile } from "@/lib/use-mobile";

interface ServerEntry {
  label: string;
  href: string;
  icon: React.ComponentType<{ className?: string }>;
  requires: "admin" | "list_users";
  divider?: boolean;
}

const ENTRIES: ServerEntry[] = [
  { label: "Providers", href: "/admin/settings#providers", icon: CloudIcon, requires: "admin" },
  { label: "Models", href: "/admin/settings#models", icon: CubeIcon, requires: "admin" },
  { label: "Skills", href: "/admin/settings#skills", icon: PuzzlePieceIcon, requires: "admin" },
  { label: "Vault", href: "/admin/settings#vault", icon: KeyIcon, requires: "admin" },
  { label: "Search", href: "/admin/settings#search", icon: MagnifyingGlassIcon, requires: "admin" },
  { label: "Voice", href: "/admin/settings#voice", icon: MicrophoneIcon, requires: "admin" },
  { label: "Browser", href: "/admin/settings#browser", icon: GlobeAltIcon, requires: "admin" },
  { label: "Sandbox", href: "/admin/settings#sandbox", icon: BeakerIcon, requires: "admin" },
  { label: "Users", href: "/admin/settings#users", icon: UsersIcon, requires: "list_users" },
  { label: "Server Settings", href: "/admin/settings", icon: Cog6ToothIcon, requires: "admin", divider: true },
];

export function ServerDropdown() {
  const hashNavigate = useHashNavigate();
  const { user } = useAuth();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const mobile = useMobile();

  const navigate = (href: string) => {
    hashNavigate(href);
    setOpen(false);
  };

  const isAdmin = user?.permissions?.is_admin === true;
  const canListUsers = user?.permissions?.list_users === true;

  const visible = ENTRIES.filter((e) =>
    e.requires === "admin" ? isAdmin : canListUsers,
  );

  useEffect(() => {
    if (!open || mobile) return;
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open, mobile]);

  if (visible.length === 0) return null;

  const list = (
    <>
      <div className="flex items-center justify-between px-4 py-2 border-b border-border shrink-0">
        <span className="text-sm font-medium text-text-secondary">Administration</span>
        {mobile && (
          <button
            onClick={() => setOpen(false)}
            className="flex items-center justify-center h-8 w-8 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition"
          >
            <XMarkIcon className="h-5 w-5" />
          </button>
        )}
      </div>
      <div className="flex-1 overflow-y-auto py-1">
        {visible.map((entry) => {
          const Icon = entry.icon;
          return (
            <div key={entry.href}>
              {entry.divider && <div className="border-t border-border my-1" />}
              <button
                onClick={() => navigate(entry.href)}
                className="w-full flex items-center gap-2 px-4 py-2 text-sm text-text-secondary hover:bg-surface-tertiary hover:text-text-primary transition"
              >
                <Icon className="h-4 w-4" />
                {entry.label}
              </button>
            </div>
          );
        })}
      </div>
    </>
  );

  if (mobile) {
    return (
      <>
        <button
          onClick={() => setOpen(true)}
          className="relative flex items-center justify-center h-10 w-10 rounded-full bg-surface-tertiary text-text-secondary hover:brightness-125 transition cursor-pointer"
          title="Administration"
        >
          <AdjustmentsVerticalIcon className="h-5 w-5" />
        </button>
        {open && (
          <>
            <div className="fixed inset-0 z-[69] bg-black/40" onClick={() => setOpen(false)} />
            <div className="fixed inset-y-0 right-0 z-[70] w-[85vw] max-w-sm bg-surface shadow-xl flex flex-col transition-transform duration-200 ease-out">
              {list}
            </div>
          </>
        )}
      </>
    );
  }

  return (
    <div ref={ref} className="relative flex items-center">
      <button
        onClick={() => setOpen((v) => !v)}
        className={`relative flex items-center justify-center h-10 w-10 transition cursor-pointer ${
          open
            ? "rounded-t-xl rounded-b-none bg-surface-secondary text-text-primary z-[61] border border-border border-b-0"
            : "rounded-full bg-surface-tertiary text-text-secondary hover:brightness-125"
        }`}
        title="Administration"
      >
        <AdjustmentsVerticalIcon className="h-5 w-5" />
      </button>

      {open && (
        <div className="absolute right-0 top-full z-[60] w-56 rounded-xl rounded-tr-none border border-border bg-surface-secondary shadow-lg">
          <div className="absolute -top-px right-0 w-[calc(theme(spacing.10)-3px)] h-[2px] bg-surface-secondary z-[60]" />
          <div className="pb-1">{list}</div>
        </div>
      )}
    </div>
  );
}
