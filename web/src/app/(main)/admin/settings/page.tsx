"use client";

import { useState, useEffect, useCallback, useMemo } from "react";
import { useRouter } from "next/navigation";
import { XMarkIcon } from "@heroicons/react/24/outline";
import { useAuth } from "@/lib/auth";
import { useMobile } from "@/lib/use-mobile";
import { useNavigation } from "@/lib/navigation-context";
import { RestartBanner } from "@/components/settings/restart-banner";
import { SettingsProvider } from "@/components/settings/settings-context";
import type { SectionHandlers } from "@/components/settings/settings-context";
import { UsersSection } from "@/components/settings/sections/users-section";
import { ProvidersSection } from "@/components/settings/sections/providers-section";
import { ModelsSection } from "@/components/settings/sections/models-section";
import { ServerSection } from "@/components/settings/sections/server-section";
import { TimezoneSection } from "@/components/settings/sections/timezone-section";
import { AuthSection } from "@/components/settings/sections/auth-section";
import { MailSection } from "@/components/settings/sections/mail-section";
import { SsoSection } from "@/components/settings/sections/sso-section";
import { BrowserSection } from "@/components/settings/sections/browser-section";
import { SearchSection } from "@/components/settings/sections/search-section";
import { MemorySection } from "@/components/settings/sections/memory-section";
import { VoiceSection } from "@/components/settings/sections/voice-section";
import { ServerVaultSection } from "@/components/settings/sections/vault-section";
import { AdvancedSection } from "@/components/settings/sections/advanced-section";
import { SkillsSection } from "@/components/settings/sections/skills-section";
import { SandboxSettingsSection } from "@/components/settings/sections/sandbox-section";
import { AgentsSection } from "@/components/settings/sections/agents-section";
import { LogsSection } from "@/components/settings/sections/logs-section";
import { CostReportsSection } from "@/components/settings/sections/cost-reports-section";
import { getConfigDocument, updateConfig } from "@/lib/config-types";
import type { Config, ConfigUpdateResponse } from "@/lib/config-types";
import { modelGroupsPatch } from "@/lib/model-authoring";
import { acceptProviderDrafts, type ProviderDrafts } from "@/lib/provider-drafts";

const TABS = [
  { id: "providers", label: "Providers", saveable: true, divider: false },
  { id: "models", label: "Models", saveable: true, divider: false },
  { id: "agents", label: "Agents", saveable: false, divider: false },
  { id: "memory", label: "Memory", saveable: true, divider: true },
  { id: "skills", label: "Skills", saveable: false, divider: false },
  { id: "search", label: "Search", saveable: true, divider: false },
  { id: "voice", label: "Voice", saveable: true, divider: false },
  { id: "browser", label: "Browser", saveable: true, divider: false },
  { id: "vault", label: "Vault", saveable: true, divider: true },
  { id: "sandbox", label: "Sandbox", saveable: true, divider: false },
  { id: "auth", label: "Authentication", saveable: true, divider: true },
  { id: "sso", label: "Single Sign-On", saveable: true, divider: false },
  { id: "mail", label: "Email", saveable: true, divider: false },
  { id: "users", label: "Users", saveable: false, divider: false },
  { id: "costs", label: "Costs", saveable: false, divider: false },
  { id: "timezone", label: "Timezone", saveable: true, divider: true },
  { id: "server", label: "Server", saveable: true, divider: false },
  { id: "advanced", label: "Advanced", saveable: true, divider: false },
  { id: "logs", label: "Logs", saveable: false, divider: true },
] as const;

type TabId = (typeof TABS)[number]["id"];

export default function AdminSettingsPage() {
  const router = useRouter();
  const { user } = useAuth();
  const isAdmin = user?.permissions?.is_admin === true;
  const canListUsers = user?.permissions?.list_users === true;
  const canViewCosts = user?.permissions?.view_usage_analytics === true;
  const hasAccess = isAdmin || canListUsers || canViewCosts;

  useEffect(() => {
    if (user && !hasAccess) router.replace("/settings");
  }, [user, hasAccess, router]);

  const [config, setConfig] = useState<Config | null>(null);
  // The config as last persisted (loaded, or after a successful save) - used
  // to diff model-group edits into a minimal patch (`modelGroupsPatch`) and to
  // tell `ModelsSection` which provider connections actually have a saved,
  // usable credential versus an unsaved draft edit.
  const [savedConfig, setSavedConfig] = useState<Config | null>(null);
  // The backend the server booted with - snapshot at load so it stays put while
  // the user edits the draft; it's what carries the "Active" badge (a backend
  // switch only takes effect after a restart + reload).
  const [activeBackend, setActiveBackend] = useState<Config["memory"]["backend"] | null>(null);
  const [patch, setPatch] = useState<Record<string, unknown>>({});
  const [persistedRevision, setPersistedRevision] = useState("");
  const [providerDrafts, setProviderDrafts] = useState<ProviderDrafts>({});
  const [modelBlock, setModelBlock] = useState<string | null>(null);
  const [providerBlock, setProviderBlock] = useState<string | null>(null);
  // Bumped whenever the provider form's own state (drafts, per-card local
  // state) must be thrown away - after a save/load/discard - by remounting
  // both `ProvidersSection` and `ModelsSection` (which reads provider configs).
  const [providerFormEpoch, setProviderFormEpoch] = useState(0);
  const [activeTab, setActiveTabState] = useState<TabId>(() => {
    if (typeof window !== "undefined") {
      const hash = window.location.hash.slice(1);
      if (TABS.some((t) => t.id === hash)) return hash as TabId;
    }
    if (isAdmin) return "providers";
    return canListUsers ? "users" : "costs";
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

  const [saving, setSaving] = useState(false);
  const [showRestart, setShowRestart] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [configLoading, setConfigLoading] = useState(true);
  const [sectionModified, setSectionModified] = useState(false);
  const [sectionHandlers, setSectionHandlers] = useState<Map<string, SectionHandlers>>(new Map());

  const activeTabDef = TABS.find((t) => t.id === activeTab);
  const isSaveableTab = activeTabDef?.saveable === true;
  const isConfigTab = isSaveableTab;

  const loadConfig = useCallback(async () => {
    if (!isAdmin) {
      setConfigLoading(false);
      return;
    }
    setConfigLoading(true);
    setError(null);
    try {
      const document = await getConfigDocument();
      setConfig(document.config);
      setSavedConfig(document.config);
      setPersistedRevision(document.persisted_revision);
      setShowRestart(document.restart_required);
      setProviderDrafts({});
      setProviderFormEpoch((epoch) => epoch + 1);
      setActiveBackend(document.config.memory?.backend ?? null);
    } catch (err) {
      // The server names the file and the field when config.yaml can't be
      // read (and 422s rather than dying mid-response), so show what it said:
      // "Failed to load configuration" alone leaves an operator with a dead
      // settings page and nothing to act on.
      setConfig(null);
      setError(
        err instanceof Error && err.message ? err.message : "Failed to load configuration"
      );
    } finally {
      setConfigLoading(false);
    }
  }, [isAdmin]);

  useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  const hasPendingChanges = Object.keys(patch).length > 0 || sectionModified;

  const handleSave = useCallback(async () => {
    if (!hasPendingChanges) return;
    setSaving(true);
    setError(null);
    try {
      // A provider connection with an unresolved block reason (still
      // validating, failed validation, no accepted credential yet) must not
      // reach the config save - `providerBlock` names why.
      if (patch.providers && providerBlock) throw new Error(providerBlock);
      if (patch.models && modelBlock) throw new Error(modelBlock);
      if (Object.keys(patch).length > 0) {
        const acceptedPatch = await acceptProviderDrafts(patch, providerDrafts, (handle, connection) => {
          setConfig((previous) => (previous ? { ...previous, providers: { ...previous.providers, [handle]: connection } } : previous));
          setPatch((previous) => ({ ...previous, providers: { ...(previous.providers as Record<string, unknown>), [handle]: connection } }));
          setProviderDrafts((previous) => {
            const next = { ...previous };
            delete next[handle];
            return next;
          });
        });
        const result = await updateConfig(acceptedPatch, { expectedPersistedRevision: persistedRevision, baseline: savedConfig });
        setConfig(result.config);
        setSavedConfig(result.config);
        setPersistedRevision(result.persisted_revision);
        setProviderDrafts({});
        setProviderFormEpoch((epoch) => epoch + 1);
        setPatch({});
        if (result.restart_required) setShowRestart(true);
      }
      for (const handler of sectionHandlers.values()) {
        await handler.save();
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  }, [patch, hasPendingChanges, sectionHandlers, persistedRevision, providerBlock, modelBlock, providerDrafts, savedConfig]);

  const handleDiscard = useCallback(() => {
    setProviderFormEpoch((epoch) => epoch + 1);
    setPatch({});
    loadConfig();
    for (const handler of sectionHandlers.values()) {
      handler.discard();
    }
  }, [loadConfig, sectionHandlers]);

  const handleRefresh = useCallback(async () => {
    setPatch({});
    await loadConfig();
  }, [loadConfig]);

  const updatePatch = useCallback((section: string, value: unknown) => {
    setPatch((prev) => ({ ...prev, [section]: value }));
    setConfig((prev) => prev ? { ...prev, [section]: value } as Config : prev);
  }, []);

  const updateModels = useCallback((models: Config["models"]) => {
    setPatch(previous => {
      const next = { ...previous };
      const changes = modelGroupsPatch(savedConfig?.models ?? {}, models);
      if (Object.keys(changes).length) next.models = changes;
      else delete next.models;
      return next;
    });
    setConfig(previous => previous ? { ...previous, models } : previous);
  }, [savedConfig]);

  const updateProviders = useCallback((providers: Config["providers"], removed: string[] = []) => {
    setPatch((previous) => {
      const value: Record<string, unknown> = { ...(previous.providers as Record<string, unknown> ?? {}), ...providers };
      for (const handle of removed) value[handle] = null;
      return { ...previous, providers: value };
    });
    setConfig((previous) => previous ? { ...previous, providers } : previous);
  }, []);

  const providerSaved = useCallback((result: ConfigUpdateResponse) => {
    setConfig(result.config);
    setSavedConfig(result.config);
    setPersistedRevision(result.persisted_revision);
    setPatch({});
    setProviderDrafts({});
    setShowRestart(result.restart_required);
    setProviderFormEpoch((epoch) => epoch + 1);
  }, []);

  const mobile = useMobile();
  const { mobileSubNavOpen: sidebarOpen, setMobileSubNavOpen: setSidebarOpen } = useNavigation();

  const visibleTabs = useMemo(() => {
    return TABS.filter((t) => {
      if (t.id === "users") return canListUsers;
      // Spend visibility is its own Cedar capability, so a custom policy can
      // grant it to a group that isn't `admins`.
      if (t.id === "costs") return canViewCosts;
      return isAdmin;
    });
  }, [canListUsers, canViewCosts, isAdmin]);

  const sidebarContent = (
    <>
      <h2 className="text-lg font-semibold text-text-primary mb-4">Server Settings</h2>
      <nav className="space-y-1 flex-1">
        {visibleTabs.map((t) => (
          <div key={t.id}>
            {t.divider && <div className="border-t border-border my-2" />}
            <button
              onClick={() => { setActiveTab(t.id); if (mobile) setSidebarOpen(false); }}
              className={`w-full text-left rounded-lg px-3 py-2 text-sm transition ${
                activeTab === t.id
                  ? "bg-accent/10 text-accent font-medium"
                  : "text-text-secondary hover:bg-surface-tertiary hover:text-text-primary"
              }`}
            >
              {t.label}
            </button>
          </div>
        ))}
      </nav>
    </>
  );

  if (user && !hasAccess) return null;

  return (
    <SettingsProvider
      onRefresh={handleRefresh}
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
                <h2 className="text-lg font-semibold text-text-primary">Server Settings</h2>
                <button
                  onClick={() => setSidebarOpen(false)}
                  className="flex items-center justify-center h-10 w-10 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition"
                >
                  <XMarkIcon className="h-5 w-5" />
                </button>
              </div>
              <nav className="space-y-1 flex-1 overflow-y-auto">
                {visibleTabs.map((t) => (
                  <div key={t.id}>
                    {t.divider && <div className="border-t border-border my-2" />}
                    <button
                      onClick={() => { setActiveTab(t.id); setSidebarOpen(false); }}
                      className={`w-full text-left rounded-lg px-3 py-2 text-sm transition ${
                        activeTab === t.id
                          ? "bg-accent/10 text-accent font-medium"
                          : "text-text-secondary hover:bg-surface-tertiary hover:text-text-primary"
                      }`}
                    >
                      {t.label}
                    </button>
                  </div>
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
            {showRestart && <RestartBanner visible={showRestart} />}

            {/* Save errors. A load failure renders its own block below, with
                the retry — showing both said the same thing twice. */}
            {error && isConfigTab && config && (
              <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text">{error}</div>
            )}

            <div className="min-h-[400px]">
              {activeTab === "users" && <UsersSection />}
              {activeTab === "skills" && <SkillsSection scope="shared" />}
              {activeTab === "agents" && <AgentsSection />}
              {activeTab === "logs" && <LogsSection />}
              {activeTab === "costs" && <CostReportsSection />}
              {isConfigTab && configLoading && (
                <p className="text-sm text-text-tertiary">Loading configuration...</p>
              )}

              {isConfigTab && !configLoading && !config && (
                <div className="space-y-3">
                  <p className="text-sm font-medium text-error-text">
                    {isAdmin
                      ? "Couldn't load the server configuration"
                      : "Server configuration is visible to administrators only."}
                  </p>
                  {isAdmin && (
                    <>
                      {/* The loader's message is multi-line: it names
                          data/config.yaml and, for the common mistakes, the
                          YAML to write. Keep the line breaks. */}
                      <pre className="whitespace-pre-wrap break-words rounded-lg bg-error-bg p-3 text-xs text-error-text">
                        {error || "Failed to load configuration"}
                      </pre>
                      <button
                        onClick={handleRefresh}
                        className="rounded-lg border border-border px-3 py-1.5 text-sm text-text-secondary hover:bg-surface-tertiary hover:text-text-primary transition"
                      >
                        Try again
                      </button>
                    </>
                  )}
                </div>
              )}

              {config && (
                <>
                  {activeTab === "providers" && (
                    <ProvidersSection
                      key={providerFormEpoch}
                      providers={config.providers}
                      onChange={updateProviders}
                      drafts={providerDrafts}
                      onDraftsChange={setProviderDrafts}
                      persistedRevision={persistedRevision}
                      hasUnsavedChanges={hasPendingChanges}
                      onReadyChange={setProviderBlock}
                      onSaved={providerSaved}
                    />
                  )}
                  {activeTab === "models" && (
                    <ModelsSection
                      key={providerFormEpoch}
                      models={config.models}
                      savedModels={savedConfig?.models}
                      enabledProviders={Object.entries(config.providers)
                        .filter(([, provider]) => provider.enabled !== false)
                        .map(([id]) => id)}
                      providerConfigs={config.providers}
                      savedProviderConfigs={savedConfig?.providers}
                      providerDrafts={providerDrafts}
                      onChange={updateModels}
                      onReadyChange={setModelBlock}
                    />
                  )}
                  {activeTab === "server" && (
                    <ServerSection
                      server={config.server}
                      onChange={(v) => updatePatch("server", v)}
                    />
                  )}
                  {activeTab === "timezone" && (
                    <TimezoneSection
                      server={config.server}
                      onChange={(v) => updatePatch("server", v)}
                    />
                  )}
                  {activeTab === "auth" && (
                    <AuthSection
                      auth={config.auth}
                      onChange={(v) => updatePatch("auth", v)}
                    />
                  )}
                  {activeTab === "sso" && (
                    <SsoSection
                      sso={config.sso}
                      onChange={(v) => updatePatch("sso", v)}
                      hasBaseUrl={!!(config.server.base_url || config.server.backend_url)}
                    />
                  )}
                  {activeTab === "mail" && (
                    <MailSection
                      mail={config.mail}
                      onChange={(v) => updatePatch("mail", v)}
                      hasFrontendUrl={!!(config.server.frontend_url || config.server.base_url)}
                    />
                  )}
                  {activeTab === "browser" && (
                    <BrowserSection
                      browser={config.browser}
                      onChange={(v) => updatePatch("browser", v)}
                    />
                  )}
                  {activeTab === "search" && (
                    <SearchSection
                      search={config.search}
                      onChange={(v) => updatePatch("search", v)}
                    />
                  )}
                  {activeTab === "memory" && (
                    <MemorySection
                      memory={config.memory}
                      models={config.models}
                      activeBackend={activeBackend}
                      onChange={(v) => updatePatch("memory", v)}
                    />
                  )}
                  {activeTab === "voice" && (
                    <VoiceSection
                      voice={config.voice}
                      onChange={(v) => updatePatch("voice", v)}
                    />
                  )}
                  {activeTab === "sandbox" && (
                    <SandboxSettingsSection
                      sandbox={config.sandbox}
                      onChange={(v) => updatePatch("sandbox", v)}
                    />
                  )}
                  {activeTab === "vault" && (
                    <ServerVaultSection
                      vault={config.vault}
                      onChange={(v) => updatePatch("vault", v)}
                    />
                  )}
                  {activeTab === "advanced" && (
                    <AdvancedSection
                      inference={config.inference}
                      scheduler={config.scheduler}
                      app={config.app}
                      onChange={(update) => {
                        if (update.inference) updatePatch("inference", update.inference);
                        if (update.scheduler) updatePatch("scheduler", update.scheduler);
                        if (update.app) updatePatch("app", update.app);
                      }}
                    />
                  )}
                </>
              )}
            </div>

            {isSaveableTab && config && (
              <div className="pt-4 pb-2 border-t border-border flex items-center justify-end gap-2">
                <button
                  onClick={handleDiscard}
                  disabled={!hasPendingChanges}
                  className="w-28 rounded-lg border border-border py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary disabled:opacity-50 transition"
                >
                  Discard
                </button>
                <button
                  onClick={handleSave}
                  disabled={!hasPendingChanges || saving || !!(patch.providers && providerBlock) || !!(patch.models && modelBlock)}
                  title={((patch.providers && providerBlock) || (patch.models && modelBlock) || undefined) as string | undefined}
                  className="w-28 rounded-lg bg-accent py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
                >
                  {saving ? "Saving..." : "Save"}
                </button>
              </div>
            )}
          </div>
        </div>
      </div>
    </SettingsProvider>
  );
}
