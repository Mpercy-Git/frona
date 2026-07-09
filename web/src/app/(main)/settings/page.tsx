"use client";

import { useState, useEffect, useCallback } from "react";
import { XMarkIcon } from "@heroicons/react/24/outline";
import { useMobile } from "@/lib/use-mobile";
import { useNavigation } from "@/lib/navigation-context";
import { SettingsProvider } from "@/components/settings/settings-context";
import type { SectionHandlers } from "@/components/settings/settings-context";
import { ProfileSection } from "@/components/settings/sections/profile-section";
import { McpSection } from "@/components/settings/sections/mcp-section";
import { ChannelsSection } from "@/components/settings/sections/channels-section";
import { SkillsSection } from "@/components/settings/sections/skills-section";
import { UserVaultSection } from "@/components/settings/sections/vault-section";
import { NotificationsSection } from "@/components/settings/sections/notifications-section";

const TABS = [
  { id: "profile", label: "Profile" },
  { id: "notifications", label: "Notifications" },
  { id: "channels", label: "Channels" },
  { id: "skills", label: "Skills" },
  { id: "mcp", label: "MCP" },
  { id: "vault", label: "Vault" },
] as const;

type TabId = (typeof TABS)[number]["id"];

export default function SettingsPage() {
  const [activeTab, setActiveTabState] = useState<TabId>(() => {
    if (typeof window !== "undefined") {
      const hash = window.location.hash.slice(1);
      if (TABS.some((t) => t.id === hash)) return hash as TabId;
    }
    return "profile";
  });

  const setActiveTab = useCallback((tab: TabId) => {
    setActiveTabState(tab);
    window.history.replaceState(null, "", `#${tab}`);
  }, []);

  useEffect(() => {
    const sync = () => {
      const hash = window.location.hash.slice(1);
      if (TABS.some((t) => t.id === hash)) setActiveTabState(hash as TabId);
    };
    sync();
    window.addEventListener("hashchange", sync);
    return () => window.removeEventListener("hashchange", sync);
  }, []);

  const [, setSectionModified] = useState(false);
  const [, setSectionHandlers] = useState<Map<string, SectionHandlers>>(new Map());

  const mobile = useMobile();
  const { mobileSubNavOpen: sidebarOpen, setMobileSubNavOpen: setSidebarOpen } = useNavigation();

  const sidebarContent = (
    <>
      <h2 className="text-lg font-semibold text-text-primary mb-4">My Settings</h2>
      <nav className="space-y-1 flex-1">
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => { setActiveTab(t.id); if (mobile) setSidebarOpen(false); }}
            className={`w-full text-left rounded-lg px-3 py-2 text-sm transition ${
              activeTab === t.id
                ? "bg-accent/10 text-accent font-medium"
                : "text-text-secondary hover:bg-surface-tertiary hover:text-text-primary"
            }`}
          >
            {t.label}
          </button>
        ))}
      </nav>
    </>
  );

  return (
    <SettingsProvider
      onRefresh={async () => {}}
      onModifiedChange={setSectionModified}
      onHandlersChange={setSectionHandlers}
    >
      <div className="flex h-full bg-surface">
        {mobile ? (
          <>
            {sidebarOpen && (
              <div
                className="fixed inset-0 z-40 bg-black/40"
                onClick={() => setSidebarOpen(false)}
              />
            )}
            <div
              className={`fixed inset-y-0 left-0 z-50 flex flex-col w-[85vw] bg-surface-nav border-r border-border shadow-xl transition-transform duration-200 ease-out p-4 ${
                sidebarOpen ? "translate-x-0" : "-translate-x-full"
              }`}
            >
              <div className="flex items-center justify-between mb-2">
                <h2 className="text-lg font-semibold text-text-primary">My Settings</h2>
                <button
                  onClick={() => setSidebarOpen(false)}
                  className="flex items-center justify-center h-10 w-10 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition"
                >
                  <XMarkIcon className="h-5 w-5" />
                </button>
              </div>
              <nav className="space-y-1 flex-1 overflow-y-auto">
                {TABS.map((t) => (
                  <button
                    key={t.id}
                    onClick={() => { setActiveTab(t.id); setSidebarOpen(false); }}
                    className={`w-full text-left rounded-lg px-3 py-2 text-sm transition ${
                      activeTab === t.id
                        ? "bg-accent/10 text-accent font-medium"
                        : "text-text-secondary hover:bg-surface-tertiary hover:text-text-primary"
                    }`}
                  >
                    {t.label}
                  </button>
                ))}
              </nav>
            </div>
          </>
        ) : (
          <div className="border-r border-border bg-surface-nav p-4 flex flex-col" style={{ width: 289 }}>
            {sidebarContent}
          </div>
        )}

        <div className="flex-1 overflow-y-auto min-w-0">
          <div className="max-w-2xl mx-auto p-4 md:p-8 space-y-6">
            <div className="min-h-[400px]">
              {activeTab === "profile" && <ProfileSection />}
              {activeTab === "notifications" && <NotificationsSection />}
              {activeTab === "channels" && <ChannelsSection />}
              {activeTab === "skills" && <SkillsSection scope="user" />}
              {activeTab === "mcp" && <McpSection />}
              {activeTab === "vault" && <UserVaultSection />}
            </div>
          </div>
        </div>
      </div>
    </SettingsProvider>
  );
}
