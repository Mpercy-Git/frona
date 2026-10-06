"use client";

import { useState, useEffect, useCallback, Suspense } from "react";
import { useSearchParams, useRouter } from "next/navigation";
import { ArrowLeftIcon, ArrowPathIcon, CpuChipIcon, PlayIcon, StopIcon, TrashIcon, PlusIcon, InformationCircleIcon, CommandLineIcon, DocumentTextIcon, KeyIcon, Cog6ToothIcon, EyeIcon, EyeSlashIcon } from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import { SectionHeader, SectionPanel, Field, TextInput } from "@/components/settings/field";
import { formatDistanceToNow } from "date-fns";
import { SandboxSection } from "@/components/agents/configure/sandbox-section";
import { AddCredentialForm, type VaultGrant, type VaultConnection, type PendingCredential } from "@/components/agents/configure/creds-section";
import { ConfigSidebar } from "@/components/layout/config-sidebar";
import { McpLogViewer } from "@/components/mcp/mcp-log-viewer";

interface McpServer {
  id: string;
  slug: string;
  display_name: string;
  description: string | null;
  repository_url: string | null;
  registry_id: string | null;
  status: string;
  /** Why the last install or start gave up. Outlives the request that produced it. */
  last_error: string | null;
  command: string;
  args: string[];
  resolved_ref: string | null;
  server_version: string | null;
  tool_count: number;
  active_transport: string;
  transports: Array<
    { Stdio: { args: string[]; env: Record<string, string> } } |
    { Http: { args: string[]; env: Record<string, string>; port_env_var: string | null; endpoint_path: string | null; url: string | null } }
  >;
  env: Record<string, string>;
  sandbox_policy: {
    network_access?: boolean;
    network_destinations?: string[];
    read_paths?: string[];
    write_paths?: string[];
  };
  installed_at: string;
  last_started_at: string | null;
}

interface EnvVarDef {
  name: string;
  description: string | null;
  is_required: boolean;
  is_secret: boolean;
}

interface RegistryPackage {
  transport: { type: string };
  environment_variables: EnvVarDef[];
}

interface RegistryEntry {
  name: string;
  packages: RegistryPackage[];
  remotes?: Array<{ type: string; url: string }>;
}

const SECTIONS = [
  { id: "status", label: "Status" },
  { id: "prompt", label: "Prompt" },
  { id: "config", label: "Config" },
  { id: "sandbox", label: "Sandbox" },
  { id: "logs", label: "Logs" },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const STATUS_BADGE: Record<string, string> = {
  installed: "bg-surface-tertiary text-text-secondary",
  running: "bg-green-500/15 text-green-500",
  stopped: "bg-yellow-500/15 text-yellow-500",
  failed: "bg-red-500/15 text-red-500",
  starting: "bg-blue-400/15 text-blue-400",
};

/** Values under names like these are hidden until asked for, in case of shoulder-surfing. */
function looksSensitive(name: string): boolean {
  return /(KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL|AUTH)/i.test(name);
}

function sameEnv(a: Record<string, string>, b: Record<string, string>): boolean {
  const ak = Object.keys(a);
  return ak.length === Object.keys(b).length && ak.every((k) => b[k] === a[k]);
}

function McpServerPage() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const serverId = searchParams.get("id");

  const [server, setServer] = useState<McpServer | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState(false);
  const [updateNote, setUpdateNote] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  /** Set after a save that changed a running server's config, until it is restarted. */
  const [restartRequired, setRestartRequired] = useState(false);
  const [revealedEnv, setRevealedEnv] = useState<Set<string>>(new Set());
  const [envKeyError, setEnvKeyError] = useState<string | null>(null);

  const sectionParam = searchParams.get("section");
  const initialSection = SECTIONS.some((s) => s.id === sectionParam) ? (sectionParam as SectionId) : "status";
  const [activeSection, setActiveSectionState] = useState<SectionId>(initialSection);

  const setActiveSection = useCallback((section: SectionId) => {
    setActiveSectionState(section);
    const params = new URLSearchParams(searchParams.toString());
    params.set("section", section);
    router.replace(`/mcp?${params.toString()}`, { scroll: false });
  }, [router, searchParams]);

  const [description, setDescription] = useState("");
  const [envValues, setEnvValues] = useState<Record<string, string>>({});
  const [envDefsByTransport, setEnvDefsByTransport] = useState<Record<string, EnvVarDef[]>>({});
  const [grants, setGrants] = useState<Array<{ id: string; query: string; connection_id: string; connection_name: string; vault_item_id: string; item_name: string; fields: string[]; target?: { Single?: { env_var: string }; Prefix?: { env_var_prefix: string } } }>>([]);
  const [vaultGrants, setVaultGrants] = useState<VaultGrant[]>([]);
  const [vaultConnections, setVaultConnections] = useState<Map<string, VaultConnection>>(new Map());
  const [credDialogEnvVar, setCredDialogEnvVar] = useState<string | null>(null);
  const [showAddMenu, setShowAddMenu] = useState(false);
  const [credMenuVar, setCredMenuVar] = useState<string | null>(null);
  const [pendingCreds, setPendingCreds] = useState<PendingCredential[]>([]);
  const [deletedGrantIds, setDeletedGrantIds] = useState<Set<string>>(new Set());
  const [credDialogExisting, setCredDialogExisting] = useState<{ connection_id: string; vault_item_id: string } | undefined>(undefined);
  const [sandboxConfig, setSandboxConfig] = useState<{
    network_access: boolean;
    allowed_network_destinations: string[];
    shared_paths: { path: string; write: boolean }[];
  } | null>(null);

  const reload = useCallback(async () => {
    if (!serverId) return;
    try {
      const s = await api.get<McpServer>(`/api/mcp/servers/${serverId}`);

      let byTransport: Record<string, EnvVarDef[]> = {};
      if (s.registry_id) {
        try {
          const entry = await api.get<RegistryEntry>(
            `/api/mcp/registry/${encodeURIComponent(s.registry_id)}`
          );
          for (const pkg of entry?.packages ?? []) {
            const t = pkg.transport?.type ?? "stdio";
            byTransport[t] = pkg.environment_variables ?? [];
          }
        } catch { /* ignore */ }
      }

      let resolvedGrants: typeof grants = [];
      let resolvedVaultGrants: VaultGrant[] = [];
      let connMap = new Map<string, VaultConnection>();
      try {
        const [allGrants, allConns] = await Promise.all([
          api.get<VaultGrant[]>("/api/vaults/grants"),
          api.get<VaultConnection[]>("/api/vaults"),
        ]);
        connMap = new Map(allConns.map((c) => [c.id, c]));
        resolvedVaultGrants = allGrants.filter(
          (g) => g.principal?.kind === "mcp_server" && g.principal?.id === serverId
        );
        resolvedGrants = await Promise.all(
          resolvedVaultGrants.map(async (g) => {
            let itemName = g.query;
            let fields: string[] = [];
            try {
              const [items, f] = await Promise.all([
                api.get<Array<{ id: string; name: string }>>(`/api/vaults/${g.connection_id}/items?q=`),
                api.get<string[]>(`/api/vaults/${g.connection_id}/items/${g.vault_item_id}/fields`),
              ]);
              const found = items.find((i) => i.id === g.vault_item_id);
              if (found) itemName = found.name;
              fields = f;
            } catch { /* fallback to query */ }
            return {
              id: g.id,
              query: g.query,
              connection_id: g.connection_id,
              connection_name: connMap.get(g.connection_id)?.name ?? "Unknown",
              vault_item_id: g.vault_item_id,
              item_name: itemName,
              fields,
              target: g.target as { Single?: { env_var: string }; Prefix?: { env_var_prefix: string } } | undefined,
            };
          })
        );
      } catch { /* ignore */ }

      setServer(s);
      setDescription(s.description ?? "");
      setEnvValues(s.env ?? {});
      {
        const reads = s.sandbox_policy.read_paths ?? [];
        const writes = new Set(s.sandbox_policy.write_paths ?? []);
        const seen = new Set<string>();
        const shared_paths: { path: string; write: boolean }[] = [];
        for (const path of [...reads, ...(s.sandbox_policy.write_paths ?? [])]) {
          if (seen.has(path)) continue;
          seen.add(path);
          shared_paths.push({ path, write: writes.has(path) });
        }
        setSandboxConfig({
          network_access: s.sandbox_policy.network_access ?? true,
          allowed_network_destinations: s.sandbox_policy.network_destinations ?? [],
          shared_paths,
        });
      }
      setEnvDefsByTransport(byTransport);
      setVaultConnections(connMap);
      setVaultGrants(resolvedVaultGrants);
      setGrants(resolvedGrants);
    } catch {
      setError("Failed to load server");
    } finally {
      setLoading(false);
    }
  }, [serverId]);

  useEffect(() => {
    reload();
  }, [reload]);

  const save = async () => {
    if (!server) return;
    setSaving(true);
    setError(null);
    try {
      const extra_env: Record<string, string> = {};
      for (const [k, v] of Object.entries(envValues)) {
        if (v.trim()) extra_env[k] = v.trim();
      }
      // The server replaces its env with whatever is sent, so the map goes
      // whenever it differs - including when it is now empty, which is how
      // the last variable gets removed.
      const envChanged = !sameEnv(extra_env, server.env ?? {});
      const res = await api.patch<{ restart_required: boolean }>(`/api/mcp/servers/${serverId}`, {
        description: description !== (server.description ?? "") ? description : undefined,
        extra_env: envChanged ? extra_env : undefined,
        sandbox_policy: sandboxConfig
          ? (() => {
              const entries = (sandboxConfig.shared_paths ?? []).filter((e) => e.path);
              return {
                network_access: sandboxConfig.network_access,
                network_destinations: sandboxConfig.allowed_network_destinations.filter(Boolean),
                read_paths: entries.map((e) => e.path),
                write_paths: entries.filter((e) => e.write).map((e) => e.path),
              };
            })()
          : undefined,
      });
      for (const id of deletedGrantIds) {
        await api.delete(`/api/vaults/grants/${id}`);
      }
      for (const cred of pendingCreds) {
        await api.post("/api/vaults/grants", cred);
      }
      await reload();
      setPendingCreds([]);
      setDeletedGrantIds(new Set());
      setDirty(false);
      if (res.restart_required) setRestartRequired(true);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Save failed");
    } finally {
      setSaving(false);
    }
  };

  const start = async () => {
    setActionLoading(true);
    setError(null);
    try {
      await api.post(`/api/mcp/servers/${serverId}/start`, {});
      setRestartRequired(false);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Start failed");
    } finally {
      await reload();
      setActionLoading(false);
    }
  };

  const restart = async () => {
    setActionLoading(true);
    setError(null);
    try {
      await api.post(`/api/mcp/servers/${serverId}/stop`, {});
      await api.post(`/api/mcp/servers/${serverId}/start`, {});
      setRestartRequired(false);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Restart failed");
    } finally {
      await reload();
      setActionLoading(false);
    }
  };

  const stop = async () => {
    setActionLoading(true);
    try {
      await api.post(`/api/mcp/servers/${serverId}/stop`, {});
      await reload();
    } catch { /* ignore */ }
    finally { setActionLoading(false); }
  };

  const update = async () => {
    setActionLoading(true);
    setError(null);
    setUpdateNote(null);
    try {
      const res = await api.post<{ changed: boolean; previous_ref: string | null; restarted: boolean }>(
        `/api/mcp/servers/${serverId}/update`,
        {}
      );
      await reload();
      setUpdateNote(
        res.changed
          ? `Updated from ${res.previous_ref ?? "an unknown version"}.${res.restarted ? " Restarted." : ""}`
          : "Already up to date."
      );
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Update failed");
    } finally {
      setActionLoading(false);
    }
  };

  const uninstall = async () => {
    if (!confirm("Uninstall this MCP server? This will remove all data and credential bindings.")) return;
    setActionLoading(true);
    try {
      await api.delete(`/api/mcp/servers/${serverId}`);
      router.push("/settings#mcp");
    } catch { /* ignore */ }
    finally { setActionLoading(false); }
  };

  if (!serverId) {
    return <p className="p-8 text-sm text-error-text">No server ID provided.</p>;
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
      </div>
    );
  }

  if (!server) {
    return <p className="p-8 text-sm text-error-text">{error || "Server not found"}</p>;
  }

  const canStart = ["installed", "stopped", "failed"].includes(server.status);
  const canStop = server.status === "running";
  const ownerAvatar = server.repository_url
    ? (() => {
        const m = server.repository_url!.match(/github\.com\/([^/]+)/);
        return m ? `https://github.com/${m[1]}.png?size=64` : null;
      })()
    : null;
  const displayName = server.display_name.includes("/")
    ? server.display_name.split("/").pop()!
    : server.display_name;

  return (
    <div className="flex h-full bg-surface">
      {/* Sidebar — fixed column on desktop, drawer on mobile */}
      <ConfigSidebar
        header={
          <>
            <button
              onClick={() => router.push("/settings#mcp")}
              className="flex items-center gap-2 text-sm text-text-secondary hover:text-text-primary transition mb-4"
            >
              <ArrowLeftIcon className="h-4 w-4" />
              Back to Settings
            </button>

            <div className="flex items-center gap-2 mb-4">
              {ownerAvatar ? (
                <img src={ownerAvatar} alt="" className="h-8 w-8 rounded-lg shrink-0" />
              ) : (
                <CpuChipIcon className="h-8 w-8 text-text-tertiary shrink-0" />
              )}
              <div className="min-w-0">
                <h2 className="text-sm font-semibold text-text-primary truncate">{displayName}</h2>
                <span className={`inline-block rounded-full px-2 py-0.5 text-[10px] font-medium ${STATUS_BADGE[server.status] ?? STATUS_BADGE.installed}`}>
                  {server.status}
                </span>
              </div>
            </div>
          </>
        }
        sections={SECTIONS}
        activeSection={activeSection}
        onSelect={(id) => setActiveSection(id as SectionId)}
      />

      {/* Content */}
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-2xl mx-auto p-8 space-y-6">
          {restartRequired && server.status === "running" && (
            <div className="rounded-lg border border-yellow-500/30 bg-yellow-500/10 px-4 py-3 text-sm text-yellow-500 flex items-center justify-between gap-3">
              <span>Saved. Restart the server for the new configuration to take effect.</span>
              <button
                type="button"
                onClick={restart}
                disabled={actionLoading}
                className="inline-flex items-center gap-1.5 rounded-lg border border-yellow-500/40 px-3 py-1 text-xs font-medium hover:bg-yellow-500/10 disabled:opacity-50 transition shrink-0"
              >
                <ArrowPathIcon className="h-3.5 w-3.5" />
                {actionLoading ? "Restarting..." : "Restart now"}
              </button>
            </div>
          )}

          {activeSection === "status" && (
            <div className="space-y-6">
              <SectionHeader title="Status" description="Server information and controls" icon={InformationCircleIcon} />
              {/* The error from an action this visit, and - when there is none -
                  the one the server is still carrying from an earlier one. A
                  server found sitting at "failed" has to say why without the
                  reader going to the logs to find out. */}
              {(error || server.last_error) && (
                <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-400 flex items-start justify-between gap-3">
                  <span className="min-w-0 whitespace-pre-wrap break-words">{error ?? server.last_error}</span>
                  <button
                    type="button"
                    onClick={() => { setError(null); setActiveSection("logs"); }}
                    className="text-xs text-red-300 hover:text-red-200 underline shrink-0"
                  >
                    View logs
                  </button>
                </div>
              )}
              <div className="rounded-xl border border-border bg-surface-secondary divide-y divide-border overflow-hidden">
                {server.registry_id && (
                  <div className="px-4 py-3 flex justify-between">
                    <span className="text-sm text-text-tertiary">Registry</span>
                    <span className="text-sm text-text-primary font-mono">{server.registry_id}</span>
                  </div>
                )}
                {server.repository_url && (
                  <div className="px-4 py-3 flex justify-between">
                    <span className="text-sm text-text-tertiary">Repository</span>
                    <a
                      href={server.repository_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-sm text-accent hover:text-accent-hover"
                    >
                      {server.repository_url.replace(/^https?:\/\/(www\.)?/, "").replace(/\.git$/, "")}
                    </a>
                  </div>
                )}
                <div className="px-4 py-3 flex justify-between items-center">
                  <span className="text-sm text-text-tertiary">Transport</span>
                  {server.transports.length > 1 ? (
                    <select
                      value={server.active_transport}
                      onChange={(e) => {
                        api.patch(`/api/mcp/servers/${serverId}`, { active_transport: e.target.value })
                          .then(() => reload())
                          .catch(() => {});
                      }}
                      className="rounded-lg border border-border bg-surface px-2.5 py-1 text-sm text-text-primary"
                    >
                      {server.transports.map((t, i) => {
                        const key = "Stdio" in t ? "stdio" : "streamable-http";
                        const label = "Stdio" in t ? "STDIO" : "HTTP";
                        return <option key={i} value={key}>{label}</option>;
                      })}
                    </select>
                  ) : (
                    <span className="text-sm text-text-primary">{server.active_transport === "stdio" ? "STDIO" : "HTTP"}</span>
                  )}
                </div>
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Command</span>
                  <span className="text-sm text-text-primary font-mono truncate ml-4">{server.command} {server.args.join(" ")}</span>
                </div>
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Tools</span>
                  <span className="text-sm text-text-primary">{server.tool_count}</span>
                </div>
                {(server.resolved_ref || server.server_version) && (
                  <div className="px-4 py-3 flex justify-between">
                    <span className="text-sm text-text-tertiary">Version</span>
                    <span className="text-sm text-text-primary font-mono ml-4 truncate">
                      {server.resolved_ref ?? server.server_version}
                      {server.resolved_ref && server.server_version && server.server_version !== server.resolved_ref && (
                        <span className="text-text-tertiary"> · reports {server.server_version}</span>
                      )}
                    </span>
                  </div>
                )}
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Installed</span>
                  <span className="text-sm text-text-primary">{formatDistanceToNow(new Date(server.installed_at), { addSuffix: true })}</span>
                </div>
                {server.last_started_at && (
                  <div className="px-4 py-3 flex justify-between">
                    <span className="text-sm text-text-tertiary">Last started</span>
                    <span className="text-sm text-text-primary">{formatDistanceToNow(new Date(server.last_started_at), { addSuffix: true })}</span>
                  </div>
                )}
              </div>

              <div className="flex items-center gap-2">
                {canStart && (
                  <button
                    onClick={start}
                    disabled={actionLoading}
                    className="inline-flex items-center gap-1.5 rounded-lg bg-green-600 px-4 py-2 text-sm font-medium text-white hover:bg-green-700 disabled:opacity-50 transition"
                  >
                    <PlayIcon className="h-4 w-4" />
                    {actionLoading ? "Starting..." : "Start"}
                  </button>
                )}
                {canStop && (
                  <button
                    onClick={stop}
                    disabled={actionLoading}
                    className="inline-flex items-center gap-1.5 rounded-lg bg-yellow-600 px-4 py-2 text-sm font-medium text-white hover:bg-yellow-700 disabled:opacity-50 transition"
                  >
                    <StopIcon className="h-4 w-4" />
                    {actionLoading ? "Stopping..." : "Stop"}
                  </button>
                )}
                {/* A remote server installs nothing locally, so it carries no command and has nothing to reinstall. */}
                {server.command !== "" && (
                  <button
                    onClick={update}
                    disabled={actionLoading}
                    title="Reinstall from the same source, keeping credentials, policy and agent access"
                    className="inline-flex items-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-text-primary hover:bg-surface-tertiary disabled:opacity-50 transition"
                  >
                    <ArrowPathIcon className="h-4 w-4" />
                    {actionLoading ? "Updating..." : "Update"}
                  </button>
                )}
                <button
                  onClick={uninstall}
                  disabled={actionLoading}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-danger hover:bg-surface-tertiary disabled:opacity-50 transition"
                >
                  <TrashIcon className="h-4 w-4" />
                  Uninstall
                </button>
              </div>
              {updateNote && (
                <p className="text-xs text-text-tertiary">{updateNote}</p>
              )}
            </div>
          )}

          {activeSection === "prompt" && (
            <div className="flex flex-col h-full">
              <SectionHeader title="Prompt" description="Controls how agents discover and interact with this server" icon={DocumentTextIcon} />
              <SectionPanel className="flex-1 flex flex-col">
                <p className="text-sm text-text-tertiary">
                  This description is shown to agents in their system prompt. Write it to help agents identify when to use this MCP server.
                </p>
                <textarea
                  value={description}
                  onChange={(e) => { setDescription(e.target.value); setDirty(true); }}
                  placeholder="Describe what this server does and when agents should use it..."
                  className="flex-1 min-h-[200px] w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary font-mono placeholder:text-text-tertiary focus:border-accent focus:outline-none resize-none"
                />
              </SectionPanel>
            </div>
          )}

          {activeSection === "config" && (() => {
            const allEnvDefs = envDefsByTransport[server.active_transport] ?? envDefsByTransport["stdio"] ?? [];
            const declaredNames = new Set(allEnvDefs.map((e) => e.name));
            const customKeys = Object.keys(envValues).filter((k) => !declaredNames.has(k));
            const activeGrants = grants.filter((g) => !deletedGrantIds.has(g.id));
            const credOnlyKeys = activeGrants
              .filter((g) => !declaredNames.has(g.query) && !customKeys.includes(g.query))
              .map((g) => g.query);
            const pendingKeys = pendingCreds
              .map((p) => p.query)
              .filter((q) => !declaredNames.has(q) && !customKeys.includes(q) && !credOnlyKeys.includes(q));
            const allCustomKeys = [...customKeys, ...credOnlyKeys, ...pendingKeys];
            return (
              <div className="space-y-4">
                <SectionHeader title="Config" description="Environment variables and credentials for this server" icon={Cog6ToothIcon} />

                {allEnvDefs.length > 0 && (() => {
                  const findCommonPrefix = (a: string, b: string): string => {
                    const ap = a.split("_"), bp = b.split("_");
                    const parts: string[] = [];
                    for (let i = 0; i < Math.min(ap.length - 1, bp.length - 1); i++) {
                      if (ap[i] === bp[i]) parts.push(ap[i]);
                      else break;
                    }
                    return parts.join("_");
                  };

                  const groups = new Map<string, EnvVarDef[]>();
                  const assigned = new Set<string>();

                  for (let i = 0; i < allEnvDefs.length; i++) {
                    if (assigned.has(allEnvDefs[i].name)) continue;
                    let bestPrefix = "";
                    const members = [allEnvDefs[i]];
                    for (let j = i + 1; j < allEnvDefs.length; j++) {
                      if (assigned.has(allEnvDefs[j].name)) continue;
                      const cp = findCommonPrefix(allEnvDefs[i].name, allEnvDefs[j].name);
                      if (cp.length > bestPrefix.length) bestPrefix = cp;
                    }
                    if (bestPrefix) {
                      for (let j = i + 1; j < allEnvDefs.length; j++) {
                        if (!assigned.has(allEnvDefs[j].name) && allEnvDefs[j].name.startsWith(bestPrefix + "_")) {
                          members.push(allEnvDefs[j]);
                        }
                      }
                    }
                    if (members.length > 1) {
                      for (const m of members) assigned.add(m.name);
                      groups.set(bestPrefix, members);
                    }
                  }

                  const ungrouped = allEnvDefs.filter((ev) => !assigned.has(ev.name));
                  if (ungrouped.length > 0) groups.set("__ungrouped__", ungrouped);

                  const renderVar = (ev: EnvVarDef) => {
                    const grant = activeGrants.find((g) => g.query === ev.name || ev.name.startsWith(g.query + "_"));
                    const pending = !grant && pendingCreds.find((p) => p.query === ev.name || ev.name.startsWith(p.query + "_"));
                    const hasCred = !!grant || !!pending;
                    const itemName = grant?.item_name ?? (pending && pending.item_name) ?? "";
                    const connName = grant?.connection_name ?? (pending && pending.connection_name) ?? "";
                    const credConnId = grant?.connection_id ?? (pending && pending.connection_id);
                    const credItemId = grant?.vault_item_id ?? (pending && pending.vault_item_id);
                    return (
                    <div key={ev.name} className="space-y-1">
                      <div className="flex items-center gap-2 mb-2">
                        <span className="text-sm font-medium text-text-secondary">
                          {ev.description || ev.name}
                        </span>
                        <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] font-mono text-text-tertiary">{ev.name}</span>
                        {ev.is_required && <span className="text-[11px] text-red-400">required</span>}
                      </div>
                      {ev.is_secret ? (
                        hasCred ? (
                          <div className="relative">
                            <button
                              type="button"
                              onClick={() => setCredMenuVar((prev) => prev === ev.name ? null : ev.name)}
                              className="flex items-center gap-1.5 text-sm text-text-primary hover:text-accent transition cursor-pointer"
                            >
                              <KeyIcon className="h-3.5 w-3.5 text-text-tertiary" />
                              {itemName} <span className="text-text-tertiary">({connName})</span>
                            </button>
                            {credMenuVar === ev.name && (
                              <>
                                <div className="fixed inset-0 z-10" onClick={() => setCredMenuVar(null)} />
                                <div className="absolute left-0 top-full mt-1 z-20 rounded-lg border border-border bg-surface-secondary shadow-lg py-1 min-w-[140px]">
                                  <button
                                    onClick={() => {
                                      setCredDialogEnvVar(ev.name);
                                      setCredDialogExisting(credConnId && credItemId ? { connection_id: credConnId, vault_item_id: credItemId } : undefined);
                                      setCredMenuVar(null);
                                    }}
                                    className="w-full text-left px-3 py-2 text-sm text-text-primary hover:bg-surface-tertiary transition flex items-center gap-2"
                                  >
                                    <KeyIcon className="h-4 w-4 text-text-tertiary" />
                                    Change
                                  </button>
                                  <button
                                    onClick={() => {
                                      if (grant) {
                                        setDeletedGrantIds((prev) => new Set(prev).add(grant.id));
                                        setDirty(true);
                                      } else {
                                        setPendingCreds((prev) => prev.filter((p) => p.query !== ev.name && !ev.name.startsWith(p.query + "_")));
                                      }
                                      setCredMenuVar(null);
                                    }}
                                    className="w-full text-left px-3 py-2 text-sm text-danger hover:bg-surface-tertiary transition flex items-center gap-2"
                                  >
                                    <TrashIcon className="h-4 w-4" />
                                    Remove
                                  </button>
                                </div>
                              </>
                            )}
                          </div>
                        ) : (
                          <button
                            type="button"
                            onClick={() => {
                              setCredDialogEnvVar(ev.name);
                              setCredDialogExisting(undefined);
                            }}
                            className="text-sm transition cursor-pointer"
                          >
                            <span className="text-text-tertiary">Not set — </span><span className="text-accent hover:underline">Configure</span>
                          </button>
                        )
                      ) : (
                        <input
                          type="text"
                          value={envValues[ev.name] ?? ""}
                          onChange={(e) => {
                            setEnvValues((prev) => ({ ...prev, [ev.name]: e.target.value }));
                            setDirty(true);
                          }}
                          placeholder={ev.is_required ? "Required" : "Optional"}
                          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                        />
                      )}
                    </div>
                  );
                  };

                  return Array.from(groups.entries()).map(([key, items]) => (
                    <SectionPanel key={key} title={key === "__ungrouped__" ? undefined : key.split("_").map(w => w.charAt(0) + w.slice(1).toLowerCase()).join(" ")}>
                      {items.map(renderVar)}
                    </SectionPanel>
                  ));
                })()}

                <SectionPanel title="Custom Variables">
                  <div className="divide-y divide-border -my-1">
                  {allCustomKeys.map((key) => {
                    const grant = activeGrants.find((g) => g.query === key);
                    const pending = !grant ? pendingCreds.find((p) => p.query === key) : undefined;
                    const isCred = !!grant || !!pending;
                    return (
                    <div key={key} className="flex items-start gap-2 py-3">
                      <div className="flex-1 space-y-2">
                        <input
                          type="text"
                          defaultValue={key}
                          onBlur={(e) => {
                            const newKey = e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, "");
                            if (grant || newKey === key) return;
                            const problem = !newKey
                              ? "A variable needs a name."
                              : /^[0-9]/.test(newKey)
                                ? `${newKey} can't start with a digit.`
                                : declaredNames.has(newKey) || envValues[newKey] !== undefined || allCustomKeys.includes(newKey)
                                  ? `${newKey} is already defined.`
                                  : null;
                            if (problem) {
                              setEnvKeyError(problem);
                              e.target.value = key;
                              return;
                            }
                            setEnvKeyError(null);
                            setRevealedEnv((prev) => {
                              if (!prev.has(key)) return prev;
                              const next = new Set(prev);
                              next.delete(key);
                              next.add(newKey);
                              return next;
                            });
                            setEnvValues((prev) => {
                              const next = { ...prev };
                              next[newKey] = next[key] ?? "";
                              delete next[key];
                              return next;
                            });
                            setDirty(true);
                          }}
                          disabled={!!grant}
                          placeholder="VARIABLE_NAME"
                          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm font-medium text-text-primary font-mono placeholder:text-text-tertiary focus:border-accent focus:outline-none disabled:opacity-60"
                        />
                        {(() => {
                          if (isCred) {
                            const itemName = grant?.item_name ?? (pending && pending.item_name) ?? key;
                            const connName = grant?.connection_name ?? (pending && pending.connection_name) ?? "";
                            const credFields = grant?.fields ?? (pending && pending.fields) ?? [];
                            const credConnId = grant?.connection_id ?? (pending && pending.connection_id);
                            const credItemId = grant?.vault_item_id ?? (pending && pending.vault_item_id);
                            const target = grant?.target ?? (pending?.target as { Single?: { env_var: string }; Prefix?: { env_var_prefix: string } } | undefined);
                            let envVarLabels: string[];
                            if (target?.Single) {
                              envVarLabels = [target.Single.env_var];
                            } else {
                              envVarLabels = credFields.map((f) => `${key}${key ? "_" : ""}${f}`);
                            }
                            return (
                              <div className="space-y-1">
                                <div className="flex items-center gap-2">
                                  <KeyIcon className="h-3.5 w-3.5 text-text-tertiary shrink-0" />
                                  <span className="text-sm text-text-primary truncate">
                                    {itemName} <span className="text-text-tertiary">({connName})</span>
                                  </span>
                                  <button
                                    type="button"
                                    onClick={() => {
                                      setCredDialogEnvVar(key);
                                      setCredDialogExisting(credConnId && credItemId ? { connection_id: credConnId, vault_item_id: credItemId } : undefined);
                                    }}
                                    className="text-xs text-accent hover:underline shrink-0"
                                  >
                                    Change
                                  </button>
                                </div>
                                {envVarLabels.length > 0 && (
                                  <div className="flex flex-wrap gap-1">
                                    {envVarLabels.map((label) => (
                                      <span key={label} className="rounded-full border border-border bg-surface-tertiary px-2 py-0.5 font-mono text-[11px] text-text-tertiary">
                                        {label}
                                      </span>
                                    ))}
                                  </div>
                                )}
                              </div>
                            );
                          }
                          const masked = looksSensitive(key) && !revealedEnv.has(key);
                          return (
                            <div className="relative">
                              <input
                                type={masked ? "password" : "text"}
                                value={envValues[key] ?? ""}
                                onChange={(e) => {
                                  setEnvValues((prev) => ({ ...prev, [key]: e.target.value }));
                                  setDirty(true);
                                }}
                                placeholder="Value"
                                autoComplete="off"
                                className={`w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none font-mono ${looksSensitive(key) ? "pr-9" : ""}`}
                              />
                              {looksSensitive(key) && (
                                <button
                                  type="button"
                                  onClick={() => setRevealedEnv((prev) => {
                                    const next = new Set(prev);
                                    if (next.has(key)) next.delete(key); else next.add(key);
                                    return next;
                                  })}
                                  aria-label={masked ? `Show ${key}` : `Hide ${key}`}
                                  className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-text-tertiary hover:text-text-primary transition"
                                >
                                  {masked ? <EyeIcon className="h-4 w-4" /> : <EyeSlashIcon className="h-4 w-4" />}
                                </button>
                              )}
                            </div>
                          );
                        })()}
                      </div>
                      <button
                        onClick={() => {
                          if (grant) {
                            setDeletedGrantIds((prev) => new Set(prev).add(grant.id));
                            setDirty(true);
                          } else if (pending) {
                            setPendingCreds((prev) => prev.filter((p) => p.query !== key));
                          } else {
                            setEnvValues((prev) => { const next = { ...prev }; delete next[key]; return next; });
                            setDirty(true);
                          }
                        }}
                        className="mt-2 p-1.5 text-text-tertiary hover:text-danger transition rounded-lg hover:bg-surface-tertiary"
                      >
                        <TrashIcon className="h-4 w-4" />
                      </button>
                    </div>
                    );
                  })}
                  </div>
                  {allCustomKeys.length === 0 && (
                    <p className="text-sm text-text-tertiary">
                      No custom variables. Add one to pass extra settings to the server process, or a credential to inject a secret from a vault.
                    </p>
                  )}
                  {envKeyError && <p className="text-xs text-red-400">{envKeyError}</p>}
                  <div className="relative">
                    <button
                      onClick={() => setShowAddMenu((v) => !v)}
                      className="inline-flex items-center gap-1.5 text-xs text-accent hover:text-accent-hover transition"
                    >
                      <PlusIcon className="h-3.5 w-3.5" />
                      Add
                    </button>
                    {showAddMenu && (
                      <>
                        <div className="fixed inset-0 z-10" onClick={() => setShowAddMenu(false)} />
                        <div className="absolute left-0 bottom-full mb-1 z-20 rounded-lg border border-border bg-surface-secondary shadow-lg py-1 min-w-[160px]">
                          <button
                            onClick={() => {
                              let n = 1;
                              let key = `NEW_VAR_${n}`;
                              while (envValues[key] !== undefined || declaredNames.has(key)) { n++; key = `NEW_VAR_${n}`; }
                              setEnvValues((prev) => ({ ...prev, [key]: "" }));
                              setDirty(true);
                              setShowAddMenu(false);
                            }}
                            className="w-full text-left px-3 py-2 text-sm text-text-primary hover:bg-surface-tertiary transition flex items-center gap-2"
                          >
                            <CommandLineIcon className="h-4 w-4 text-text-tertiary" />
                            Variable
                          </button>
                          <button
                            onClick={() => {
                              setCredDialogEnvVar("");
                              setCredDialogExisting(undefined);
                              setShowAddMenu(false);
                            }}
                            className="w-full text-left px-3 py-2 text-sm text-text-primary hover:bg-surface-tertiary transition flex items-center gap-2"
                          >
                            <KeyIcon className="h-4 w-4 text-text-tertiary" />
                            Credential
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                </SectionPanel>
              </div>
            );
          })()}

          {activeSection === "sandbox" && (
            <SandboxSection
              sandbox={sandboxConfig}
              onChange={(v) => { setSandboxConfig(v); setDirty(true); }}
            />
          )}

          {activeSection === "logs" && (
            <McpLogViewer serverId={server.id} serverStatus={server.status} />
          )}

          {/* Save bar */}
          {activeSection !== "logs" && activeSection !== "status" && (
            <div className="pt-4 border-t border-border flex items-center justify-end gap-2">
              {error && <span className="mr-auto text-xs text-red-400 min-w-0 break-words">{error}</span>}
              <button
                onClick={() => { setDirty(false); setPendingCreds([]); setDeletedGrantIds(new Set()); setEnvKeyError(null); reload(); }}
                disabled={!dirty}
                className="rounded-lg border border-border px-4 py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary disabled:opacity-50 transition"
              >
                Discard
              </button>
              <button
                onClick={save}
                disabled={!dirty || saving}
                className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
              >
                {saving ? "Saving..." : "Save"}
              </button>
            </div>
          )}
        </div>
      </div>

      {credDialogEnvVar != null && serverId && (
        <div className="fixed inset-0 z-50 flex items-center justify-center">
          <div className="absolute inset-0 bg-black/50" onClick={() => setCredDialogEnvVar(null)} />
          <div className="relative w-full max-w-lg rounded-xl border border-border bg-surface-secondary p-5 shadow-xl mx-4 space-y-3">
            <AddCredentialForm
              connections={vaultConnections}
              principalKind="mcp_server"
              principalId={serverId}
              existingGrants={vaultGrants}
              targetEnvVar={credDialogEnvVar || undefined}
              initialSelection={credDialogExisting}
              onClose={() => setCredDialogEnvVar(null)}
              onCreated={() => {
                setCredDialogEnvVar(null);
                reload();
              }}
              deferred={(pending) => {
                setPendingCreds((prev) => [...prev, pending]);
                setDirty(true);
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

export default function McpPage() {
  return (
    <Suspense>
      <McpServerPage />
    </Suspense>
  );
}
