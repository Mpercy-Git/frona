"use client";

import { useState, useEffect, useCallback, useRef } from "react";
import { SectionHeader } from "../field";
import {
  CpuChipIcon,
  MagnifyingGlassIcon,
  PlayIcon,
  StopIcon,
  TrashIcon,
  ArrowDownTrayIcon,
  CheckCircleIcon,
  ExclamationTriangleIcon,
} from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import { formatDistanceToNow } from "date-fns";
import { useRouter } from "next/navigation";

interface McpServer {
  id: string;
  slug: string;
  display_name: string;
  description: string | null;
  repository_url: string | null;
  registry_id: string | null;
  status: string;
  command: string;
  args: string[];
  tool_count: number;
  installed_at: string;
  last_started_at: string | null;
}

interface Enrichment {
  github_stars: number | null;
  github_forks: number | null;
  github_pushed_at: string | null;
  github_license: string | null;
  github_primary_language: string | null;
  github_owner_avatar_url: string | null;
  github_archived: boolean | null;
}

interface EnvVarDef {
  name: string;
  description: string | null;
  is_required: boolean;
  is_secret: boolean;
}

interface RegistryPackage {
  registry_type: string;
  identifier: string;
  version: string | null;
  transport: { kind: string };
  environment_variables: EnvVarDef[];
}

interface RegistryEntry {
  name: string;
  description: string;
  version: string;
  title: string | null;
  repository: { url: string | null } | null;
  packages: RegistryPackage[];
  score: number | null;
  enrichment: Enrichment | null;
}

const STATUS_ICON: Record<string, typeof CheckCircleIcon> = {};

const STATUS_COLOR: Record<string, string> = {
  installed: "text-text-tertiary",
  running: "text-green-500",
  stopped: "text-yellow-500",
  failed: "text-red-500",
  starting: "text-blue-400",
};

export function McpSection() {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<RegistryEntry[]>([]);
  const [searching, setSearching] = useState(false);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmUninstall, setConfirmUninstall] = useState<McpServer | null>(null);
  const [confirmInstall, setConfirmInstall] = useState<RegistryEntry | null>(null);
  const router = useRouter();
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const reload = useCallback(async () => {
    try {
      const data = await api.get<McpServer[]>("/api/mcp/servers");
      setServers(data);
    } catch {
      /* ignore */
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  const handleSearch = useCallback((q: string) => {
    setQuery(q);
    setError(null);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (q.trim().length < 2) {
      setResults([]);
      return;
    }
    debounceRef.current = setTimeout(async () => {
      setSearching(true);
      try {
        const data = await api.get<RegistryEntry[]>(
          `/api/mcp/registry/search?q=${encodeURIComponent(q)}&limit=10`
        );
        setResults(data);
      } catch {
        setResults([]);
      } finally {
        setSearching(false);
      }
    }, 300);
  }, []);

  const doInstall = async (entry: RegistryEntry) => {
    setConfirmInstall(null);
    setActionLoading(entry.name);
    setError(null);
    try {
      const server = await api.post<McpServer>("/api/mcp/servers", {
        registry_id: entry.name,
      });
      setQuery("");
      setResults([]);
      await reload();
      router.push(`/mcp?id=${server.id}`);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Install failed");
    } finally {
      setActionLoading(null);
    }
  };

  const start = async (id: string) => {
    setActionLoading(id);
    setError(null);
    try {
      await api.post(`/api/mcp/servers/${id}/start`, {});
      await reload();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Start failed");
    } finally {
      setActionLoading(null);
    }
  };

  const stop = async (id: string) => {
    setActionLoading(id);
    try {
      await api.post(`/api/mcp/servers/${id}/stop`, {});
      await reload();
    } catch {
      /* ignore */
    } finally {
      setActionLoading(null);
    }
  };

  const uninstall = async (server: McpServer) => {
    setConfirmUninstall(null);
    setActionLoading(server.id);
    try {
      await api.delete(`/api/mcp/servers/${server.id}`);
      await reload();
    } catch {
      /* ignore */
    } finally {
      setActionLoading(null);
    }
  };

  const installedIds = new Set(servers.map((s) => s.registry_id).filter(Boolean));

  return (
    <div className="space-y-4">
      <SectionHeader
        title="MCP"
        description="Install and manage Model Context Protocol servers"
        icon={CpuChipIcon}
      />

      {/* Uninstall confirmation dialog */}
      {confirmUninstall && (
        <div className="fixed inset-0 z-50 flex items-center justify-center">
          <div className="absolute inset-0 bg-black/50" onClick={() => setConfirmUninstall(null)} />
          <div className="relative rounded-xl border border-border bg-surface-secondary p-4 space-y-4 max-w-lg w-full mx-4 shadow-xl">
            <div className="mb-5 pb-3 border-b border-border flex items-end justify-between gap-3">
              <div>
                <h3 className="text-lg font-semibold text-text-primary">{confirmUninstall.display_name}</h3>
                <span className="rounded-full bg-surface-tertiary px-2.5 py-0.5 text-[11px] font-medium text-text-secondary uppercase tracking-wide">uninstall</span>
              </div>
              <TrashIcon className="h-10 w-10 text-danger shrink-0" />
            </div>
            <p className="text-sm text-text-secondary">
              This will stop the server, remove all its data and credential bindings. Agents will no longer have access to its tools.
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => uninstall(confirmUninstall)}
                className="w-28 inline-flex items-center justify-center gap-1.5 rounded-lg border border-border py-2 text-sm font-medium text-danger hover:bg-surface-tertiary transition"
              >
                <TrashIcon className="h-4 w-4" />
                Uninstall
              </button>
              <button
                onClick={() => setConfirmUninstall(null)}
                className="w-28 inline-flex items-center justify-center gap-1.5 rounded-lg border border-border py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Install confirmation dialog */}
      {confirmInstall && (() => {
        const entry = confirmInstall;
        const avatar = entry.enrichment?.github_owner_avatar_url;
        const name = entry.title ?? (entry.name.includes("/") ? entry.name.split("/").pop() : entry.name);
        return (
          <div className="fixed inset-0 z-50 flex items-center justify-center">
            <div className="absolute inset-0 bg-black/50" onClick={() => setConfirmInstall(null)} />
            <div className="relative rounded-xl border border-border bg-surface-secondary p-5 space-y-4 max-w-lg w-full mx-4 shadow-xl">
              <div className="flex items-start gap-3">
                {avatar ? (
                  <img src={avatar} alt="" className="h-10 w-10 rounded-lg shrink-0" />
                ) : (
                  <CpuChipIcon className="h-10 w-10 text-text-tertiary shrink-0" />
                )}
                <div className="flex-1 min-w-0">
                  <h3 className="text-lg font-semibold text-text-primary">{name}</h3>
                  <p className="text-xs text-text-tertiary line-clamp-2">{entry.description}</p>
                </div>
              </div>
              {entry.repository?.url && (
                <a
                  href={entry.repository.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-xs text-text-tertiary hover:text-accent block"
                >
                  {entry.repository.url.replace(/^https?:\/\/(www\.)?/, "").replace(/\.git$/, "")}
                </a>
              )}
              <p className="text-sm text-text-secondary">
                This will download and install the MCP server. You can configure credentials and environment variables after installation.
              </p>
              <div className="flex gap-2">
                <button
                  onClick={() => doInstall(entry)}
                  disabled={actionLoading === entry.name}
                  className="inline-flex items-center justify-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
                >
                  <ArrowDownTrayIcon className="h-4 w-4" />
                  {actionLoading === entry.name ? "Installing..." : "Install"}
                </button>
                <button
                  onClick={() => setConfirmInstall(null)}
                  className="inline-flex items-center justify-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
                >
                  Cancel
                </button>
              </div>
            </div>
          </div>
        );
      })()}

      {/* Search */}
      <div className="relative">
        <MagnifyingGlassIcon className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-text-tertiary" />
        <input
          type="text"
          value={query}
          onChange={(e) => handleSearch(e.target.value)}
          placeholder="Search MCP servers (e.g. gmail, filesystem, github)..."
          className="w-full rounded-lg border border-border bg-surface pl-9 pr-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
        />
        {searching && (
          <div className="absolute right-3 top-1/2 -translate-y-1/2">
            <div className="h-4 w-4 animate-spin rounded-full border-2 border-accent border-t-transparent" />
          </div>
        )}
      </div>

      {error && (
        <div className="rounded-lg bg-error-bg p-3 text-sm text-error-text">{error}</div>
      )}

      {/* Search results */}
      {results.length > 0 && (
        <div className="rounded-xl border border-border bg-surface-secondary divide-y divide-border overflow-hidden">
          {results.map((entry) => {
            const alreadyInstalled = installedIds.has(entry.name);
            return (
              <div key={entry.name} className="px-4 py-3 flex items-start gap-3">
                {entry.enrichment?.github_owner_avatar_url ? (
                  <img
                    src={entry.enrichment.github_owner_avatar_url}
                    alt=""
                    className="h-8 w-8 rounded-lg shrink-0 mt-0.5"
                  />
                ) : (
                  <CpuChipIcon className="h-8 w-8 rounded-lg shrink-0 mt-0.5 text-text-tertiary" />
                )}
                <div className="flex-1 min-w-0 space-y-1">
                  <div className="text-sm font-medium text-text-primary truncate">
                    {entry.title ?? (entry.name.includes("/") ? entry.name.split("/").pop() : entry.name)}
                  </div>
                  {entry.repository?.url && (
                    <a
                      href={entry.repository.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs text-text-tertiary hover:text-accent truncate block"
                      onClick={(e) => e.stopPropagation()}
                    >
                      {entry.repository.url.replace(/^https?:\/\/(www\.)?/, "").replace(/\.git$/, "")}
                    </a>
                  )}
                  <div className="text-xs text-text-tertiary line-clamp-2">
                    {entry.description}
                  </div>
                  {entry.enrichment && (
                    <div className="flex items-center gap-3 text-xs text-text-tertiary">
                      {entry.enrichment.github_stars != null && (
                        <span className="text-sm">★ {entry.enrichment.github_stars.toLocaleString()}</span>
                      )}
                      {entry.enrichment.github_forks != null && entry.enrichment.github_forks > 0 && (
                        <span className="text-sm">⑂ {entry.enrichment.github_forks.toLocaleString()}</span>
                      )}
                      {entry.enrichment.github_primary_language && (
                        <span>{entry.enrichment.github_primary_language}</span>
                      )}
                      {entry.enrichment.github_license && (
                        <span>{entry.enrichment.github_license}</span>
                      )}
                      {entry.enrichment.github_pushed_at && (
                        <span>updated {formatDistanceToNow(new Date(entry.enrichment.github_pushed_at), { addSuffix: true })}</span>
                      )}
                      {entry.enrichment.github_archived && (
                        <span className="text-yellow-500">archived</span>
                      )}
                    </div>
                  )}
                </div>
                <div className="shrink-0">
                  {alreadyInstalled ? (
                    <CheckCircleIcon className="h-5 w-5 text-green-500" />
                  ) : (
                    <button
                      onClick={() => setConfirmInstall(entry)}
                      disabled={actionLoading === entry.name}
                      className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-surface shadow-sm hover:bg-accent-hover disabled:opacity-50 transition"
                    >
                      <ArrowDownTrayIcon className="h-3.5 w-3.5" />
                      {actionLoading === entry.name ? "Installing..." : "Install"}
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {query.trim().length >= 2 && !searching && results.length === 0 && (
        <p className="text-sm text-text-tertiary text-center py-8">No servers found</p>
      )}

      {/* Installed servers */}
      {query.trim().length < 2 && (
        <div>
          {!loading && servers.length > 0 && (
            <div className="flex items-center justify-between mb-2 min-h-[36px]">
              <h4 className="text-base font-medium text-text-secondary">Installed</h4>
            </div>
          )}
          {loading ? (
            <div className="flex items-center justify-center py-12">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
            </div>
          ) : servers.length === 0 ? (
            <p className="text-sm text-text-tertiary text-center py-8">
              No servers installed
            </p>
          ) : (
            <div className="rounded-xl border border-border bg-surface-secondary divide-y divide-border overflow-hidden">
              {servers.map((server) => {
                const isLoading = actionLoading === server.id;
                const ownerAvatar = server.repository_url
                  ? (() => {
                      const m = server.repository_url!.match(/github\.com\/([^/]+)/);
                      return m ? `https://github.com/${m[1]}.png?size=64` : null;
                    })()
                  : null;
                const canStart = ["installed", "stopped", "failed"].includes(server.status);
                const canStop = server.status === "running";
                const statusBadgeColor: Record<string, string> = {
                  created: "bg-surface-tertiary text-text-secondary",
                  running: "bg-green-500/15 text-green-500",
                  stopped: "bg-yellow-500/15 text-yellow-500",
                  failed: "bg-red-500/15 text-red-500",
                  installed: "bg-surface-tertiary text-text-secondary",
                  starting: "bg-blue-400/15 text-blue-400",
                };
                return (
                  <div
                    key={server.id}
                    onClick={(e) => { if (!(e.target as HTMLElement).closest("button")) router.push(`/mcp?id=${server.id}`); }}
                    className="px-4 py-3 flex items-center gap-3 transition hover:bg-surface-tertiary cursor-pointer"
                  >
                    {ownerAvatar ? (
                      <img src={ownerAvatar} alt="" className="h-8 w-8 rounded-lg shrink-0" />
                    ) : (
                      <CpuChipIcon className="h-8 w-8 shrink-0 text-text-tertiary" />
                    )}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium text-text-primary truncate">
                          {server.display_name.includes("/") ? server.display_name.split("/").pop() : server.display_name}
                        </span>
                        <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${statusBadgeColor[server.status] ?? "bg-surface-tertiary text-text-secondary"}`}>
                          {server.status}
                        </span>
                        {server.tool_count > 0 && (
                          <span className="text-xs text-text-tertiary">
                            {server.tool_count} tool{server.tool_count !== 1 ? "s" : ""}
                          </span>
                        )}
                      </div>
                      {server.description && (
                        <div className="text-xs text-text-tertiary line-clamp-2">{server.description}</div>
                      )}
                    </div>
                    <div className="flex items-center gap-1 shrink-0">
                      {canStart && (
                        <button
                          onClick={() => start(server.id)}
                          disabled={isLoading}
                          title="Start"
                          className="rounded-lg p-1.5 text-green-500 hover:bg-green-500/10 disabled:opacity-50 transition"
                        >
                          <PlayIcon className="h-5 w-5" />
                        </button>
                      )}
                      {canStop && (
                        <button
                          onClick={() => stop(server.id)}
                          disabled={isLoading}
                          title="Stop"
                          className="rounded-lg p-1.5 text-yellow-500 hover:bg-yellow-500/10 disabled:opacity-50 transition"
                        >
                          <StopIcon className="h-5 w-5" />
                        </button>
                      )}
                      <button
                        onClick={() => setConfirmUninstall(server)}
                        disabled={isLoading}
                        title="Uninstall"
                        className="rounded-lg p-1.5 text-text-tertiary hover:text-danger hover:bg-danger/10 disabled:opacity-50 transition"
                      >
                        <TrashIcon className="h-5 w-5" />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
