"use client";

import { useEffect } from "react";
import { ArrowLeftIcon } from "@heroicons/react/24/outline";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  /** Sub-text under the title (default tone only). */
  description?: string;
  /** Small uppercase pill under the title (danger tone). */
  badge?: string;
  /** Icon for the header. Default tone: rendered left of title. Danger tone: rendered right, in danger color. */
  icon?: React.ComponentType<{ className?: string }>;
  /** Avatar URL takes precedence over `icon` for the default-tone left element. */
  avatar?: string | null;
  tone?: "default" | "danger";
  /** When set, renders a back-arrow button before the title. */
  onBack?: () => void;
  /** Tailwind max-width class for the dialog card. */
  maxWidth?: string;
  children: React.ReactNode;
}

export function Dialog({
  open,
  onClose,
  title,
  description,
  badge,
  icon: Icon,
  avatar,
  tone = "default",
  onBack,
  maxWidth = "max-w-lg",
  children,
}: DialogProps) {
  useEffect(() => {
    if (!open) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      <div className={`relative rounded-xl border border-border bg-surface-secondary p-5 space-y-4 ${maxWidth} w-full mx-4 shadow-xl`}>
        {tone === "danger" ? (
          <div className="mb-5 pb-3 border-b border-border flex items-end justify-between gap-3">
            <div className="flex items-center gap-2 min-w-0">
              {onBack && (
                <button
                  onClick={onBack}
                  className="flex items-center justify-center h-8 w-8 -ml-2 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition"
                >
                  <ArrowLeftIcon className="h-4 w-4" />
                </button>
              )}
              <div className="min-w-0">
                <h3 className="text-lg font-semibold text-text-primary truncate">{title}</h3>
                {badge && (
                  <span className="rounded-full bg-surface-tertiary px-2.5 py-0.5 text-[11px] font-medium text-text-secondary uppercase tracking-wide">
                    {badge}
                  </span>
                )}
              </div>
            </div>
            {Icon && <Icon className="h-10 w-10 text-danger shrink-0" />}
          </div>
        ) : (
          <div className="flex items-start gap-3">
            {onBack && (
              <button
                onClick={onBack}
                className="flex items-center justify-center h-8 w-8 -ml-1 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition shrink-0"
              >
                <ArrowLeftIcon className="h-4 w-4" />
              </button>
            )}
            {avatar ? (
              <img src={avatar} alt="" className="h-10 w-10 rounded-lg shrink-0" />
            ) : Icon ? (
              <Icon className="h-10 w-10 text-text-tertiary shrink-0" />
            ) : null}
            <div className="flex-1 min-w-0">
              <h3 className="text-lg font-semibold text-text-primary truncate">{title}</h3>
              {description && (
                <p className="text-xs text-text-tertiary line-clamp-2">{description}</p>
              )}
            </div>
          </div>
        )}
        {children}
      </div>
    </div>
  );
}
