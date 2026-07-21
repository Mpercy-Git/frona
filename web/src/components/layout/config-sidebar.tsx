"use client";

import type { ReactNode } from "react";
import { XMarkIcon } from "@heroicons/react/24/outline";
import { useMobile } from "@/lib/use-mobile";
import { useNavigation } from "@/lib/navigation-context";

interface SidebarSection {
  id: string;
  label: string;
}

interface ConfigSidebarProps {
  /** Header block (back button, title, status badges) rendered above the nav. */
  header: ReactNode;
  sections: readonly SidebarSection[];
  activeSection: string;
  onSelect: (id: string) => void;
}

/**
 * Left configuration sidebar for the agent / channel / MCP detail pages.
 *
 * On desktop it's a fixed 289px column. Below the mobile breakpoint it becomes
 * an off-canvas drawer toggled by the top-bar hamburger (via the navigation
 * context's `mobileSubNavOpen`), mirroring the Settings pages — so the panel
 * stops crushing page content on small screens.
 */
export function ConfigSidebar({ header, sections, activeSection, onSelect }: ConfigSidebarProps) {
  const mobile = useMobile();
  const { mobileSubNavOpen, setMobileSubNavOpen } = useNavigation();

  const nav = (
    <nav className="space-y-1 flex-1 min-h-0 overflow-y-auto">
      {sections.map((s) => (
        <button
          key={s.id}
          onClick={() => {
            onSelect(s.id);
            if (mobile) setMobileSubNavOpen(false);
          }}
          className={`w-full text-left rounded-lg px-3 py-2 text-sm transition ${
            activeSection === s.id
              ? "bg-accent/10 text-accent font-medium"
              : "text-text-secondary hover:bg-surface-tertiary hover:text-text-primary"
          }`}
        >
          {s.label}
        </button>
      ))}
    </nav>
  );

  if (!mobile) {
    return (
      <div
        className="border-r border-border bg-surface-nav p-4 flex flex-col shrink-0"
        style={{ width: 289 }}
      >
        {header}
        {nav}
      </div>
    );
  }

  return (
    <>
      {mobileSubNavOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/40"
          onClick={() => setMobileSubNavOpen(false)}
        />
      )}
      <div
        className={`fixed inset-y-0 left-0 z-50 flex flex-col w-[85vw] max-w-sm bg-surface-nav border-r border-border shadow-xl transition-transform duration-200 ease-out p-4 ${
          mobileSubNavOpen ? "translate-x-0" : "-translate-x-full"
        }`}
      >
        <div className="flex items-start justify-between gap-2 mb-2">
          <div className="min-w-0 flex-1">{header}</div>
          <button
            onClick={() => setMobileSubNavOpen(false)}
            className="shrink-0 flex items-center justify-center h-10 w-10 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition"
          >
            <XMarkIcon className="h-5 w-5" />
          </button>
        </div>
        {nav}
      </div>
    </>
  );
}
