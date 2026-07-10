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
  PlusIcon,
  XMarkIcon,
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

  // Custom (non-registry) install form
  const [showCustom, setShowCustom] = useState(false);
  const [installingCustom, setInstallingCustom] = useState(false);
  const [customName, setCustomName] = useState("");
  const [customDescription, setCustomDescription] = useState("");
  const [customRuntime, setCustomRuntime] = useState<"npm" | "pypi">("npm");
  const [customIdentifier, setCustomIdentifier] = useState("");
  const [customVersion, setCustomVersion] = useState("");
  const [customTransport, setCustomTransport] = useState<"stdio" | "streamable-http" | "sse">("stdio");
  const [customUrl, setCustomUrl] = useState("");
  const [customEnv, setCustomEnv] = useState<{ key: string; value: string }[]>([]);

  const resetCustomForm = () => {
    setCustomName("");
    setCustomDescription("");
    setCustomRuntime("npm");
    setCustomIdentifier("");
    setCustomVersion("");
    setCustomTransport("stdio");
    setCustomUrl("");
    setCustomEnv([]);
  };

  const closeCustom = () => {
    setShowCustom(false);
    resetCustomForm();
  };

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

  const installCustom = async () => {
    const name = customName.trim();
    const identifier = customIdentifier.trim();
    if (!name || !identifier) {
      setError("Name and package identifier are required.");
      return;
    }
    setInstallingCustom(true);
    setError(null);
    try {
      const version = customVersion.trim();
      const url = customUrl.trim();
      const pkg: Record<string, unknown> = {
        registry_type: customRuntime,
        identifier,
        transport: {
          type: customTransport,
          ...(customTransport !== "stdio" && url ? { url } : {}),
        },
      };
      if (version) pkg.version = version;

      const manifest = {
        name,
        description: customDescription.trim(),
        version: version || "0.0.0",
        packages: [pkg],
      };

      const extraEnv = Object.fromEntries(
        customEnv
          .map((e) => [e.key.trim(), e.value] as const)
          .filter(([k]) => k.length > 0)
      );

      const server = await api.post<McpServer>("/api/mcp/servers", {
        manifest,
        ...(Object.keys(extraEnv).length > 0 ? { extra_env: extraEnv } : {}),
      });
      closeCustom();
      await reload();
      router.push(`/mcp?id=${server.id}`);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Install failed");
    } finally {
      setInstallingCustom(false);
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
            <div className="flex gap-2 pt-4">
              <button
                onClick={() => uninstall(confirmUninstall)}
                className="w-32 inline-flex items-center justify-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-danger hover:bg-surface-tertiary transition"
              >
                <TrashIcon className="h-4 w-4" />
                Uninstall
              </button>
              <button
                onClick={() => setConfirmUninstall(null)}
                className="w-32 inline-flex items-center justify-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
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
              <div className="flex gap-2 pt-4">
                <button
                  onClick={() => doInstall(entry)}
                  disabled={actionLoading === entry.name}
                  className="w-32 inline-flex items-center justify-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
                >
                  <ArrowDownTrayIcon className="h-4 w-4" />
                  {actionLoading === entry.name ? "Installing..." : "Install"}
                </button>
                <button
                  onClick={() => setConfirmInstall(null)}
                  className="w-32 inline-flex items-center justify-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
                >
                  Cancel
                </button>
              </div>
            </div>
          </div>
        );
      })()}

      {/* Custom install dialog */}
      {showCustom && (
        <div className="fixed inset-0 z-50 flex items-center justify-center">
          <div className="absolute inset-0 bg-black/50" onClick={closeCustom} />
          <div className="relative rounded-xl border border-border bg-surface-secondary p-5 space-y-4 max-w-lg w-full mx-4 shadow-xl max-h-[85vh] overflow-y-auto">
            <div className="flex items-start justify-between gap-3 border-b border-border pb-3">
              <div className="flex items-center gap-2">
                <CpuChipIcon className="h-6 w-6 text-text-tertiary shrink-0" />
                <h3 className="text-lg font-semibold text-text-primary">Add a custom MCP server</h3>
              </div>
              <button
                onClick={closeCustom}
                className="flex items-center justify-center h-8 w-8 rounded-lg text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition"
              >
                <XMarkIcon className="h-5 w-5" />
              </button>
            </div>

            <p className="text-xs text-text-tertiary">
              Install a server that isn&apos;t in the registry by pointing at an npm or PyPI package.
            </p>

            <div>
              <label className="block text-xs font-medium text-text-tertiary mb-1">Name</label>
              <input
                type="text"
                value={customName}
                onChange={(e) => setCustomName(e.target.value)}
                placeholder="My Custom Server"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
            </div>

            <div>
              <label className="block text-xs font-medium text-text-tertiary mb-1">Description <span className="text-text-tertiary/70">(optional)</span></label>
              <input
                type="text"
                value={customDescription}
                onChange={(e) => setCustomDescription(e.target.value)}
                placeholder="What this server does"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
            </div>

            <div className="grid grid-cols-3 gap-2">
              <div>
                <label className="block text-xs font-medium text-text-tertiary mb-1">Runtime</label>
                <select
                  value={customRuntime}
                  onChange={(e) => setCustomRuntime(e.target.value as "npm" | "pypi")}
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
                >
                  <option value="npm">npm</option>
                  <option value="pypi">PyPI</option>
                </select>
              </div>
              <div className="col-span-2">
                <label className="block text-xs font-medium text-text-tertiary mb-1">Package identifier</label>
                <input
                  type="text"
                  value={customIdentifier}
                  onChange={(e) => setCustomIdentifier(e.target.value)}
                  placeholder={customRuntime === "npm" ? "@scope/my-mcp-server" : "my-mcp-server"}
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                />
              </div>
            </div>

            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="block text-xs font-medium text-text-tertiary mb-1">Version <span className="text-text-tertiary/70">(optional)</span></label>
                <input
                  type="text"
                  value={customVersion}
                  onChange={(e) => setCustomVersion(e.target.value)}
                  placeholder="latest"
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-text-tertiary mb-1">Transport</label>
                <select
                  value={customTransport}
                  onChange={(e) => setCustomTransport(e.target.value as "stdio" | "streamable-http" | "sse")}
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
                >
                  <option value="stdio">stdio</option>
                  <option value="streamable-http">streamable-http</option>
                  <option value="sse">sse</option>
                </select>
              </div>
            </div>

            {customTransport !== "stdio" && (
              <div>
                <label className="block text-xs font-medium text-text-tertiary mb-1">URL <span className="text-text-tertiary/70">(optional)</span></label>
                <input
                  type="text"
                  value={customUrl}
                  onChange={(e) => setCustomUrl(e.target.value)}
                  placeholder="http://localhost:3000/mcp"
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                />
              </div>
            )}

            <div>
              <div className="flex items-center justify-between mb-1">
                <label className="block text-xs font-medium text-text-tertiary">Environment variables</label>
                <button
                  onClick={() => setCustomEnv((prev) => [...prev, { key: "", value: "" }])}
                  className="inline-flex items-center gap-1 text-xs text-text-secondary hover:text-accent transition"
                >
                  <PlusIcon className="h-3.5 w-3.5" />
                  Add
                </button>
              </div>
              {customEnv.length === 0 ? (
                <p className="text-xs text-text-tertiary/70">None. Add key/value pairs the server needs at runtime.</p>
              ) : (
                <div className="space-y-2">
                  {customEnv.map((row, i) => (
                    <div key={i} className="flex items-center gap-2">
                      <input
                        type="text"
                        value={row.key}
                        onChange={(e) => setCustomEnv((prev) => prev.map((r, j) => (j === i ? { ...r, key: e.target.value } : r)))}
                        placeholder="API_KEY"
                        className="w-2/5 rounded-lg border border-border bg-surface px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                      />
                      <input
                        type="text"
                        value={row.value}
                        onChange={(e) => setCustomEnv((prev) => prev.map((r, j) => (j === i ? { ...r, value: e.target.value } : r)))}
                        placeholder="value"
                        className="flex-1 rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                      />
                      <button
                        onClick={() => setCustomEnv((prev) => prev.filter((_, j) => j !== i))}
                        className="rounded-lg p-1.5 text-text-tertiary hover:text-danger hover:bg-danger/10 transition"
                        title="Remove"
                      >
                        <TrashIcon className="h-4 w-4" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="flex gap-2 pt-4">
              <button
                onClick={installCustom}
                disabled={installingCustom || !customName.trim() || !customIdentifier.trim()}
                className="w-32 inline-flex items-center justify-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
              >
                <ArrowDownTrayIcon className="h-4 w-4" />
                {installingCustom ? "Installing..." : "Install"}
              </button>
              <button
                onClick={closeCustom}
                className="w-32 inline-flex items-center justify-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}

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

      {/* Custom install trigger */}
      <div className="flex justify-end">
        <button
          onClick={() => { setError(null); setShowCustom(true); }}
          className="inline-flex items-center gap-1.5 text-sm text-text-secondary hover:text-accent transition"
        >
          <PlusIcon className="h-4 w-4" />
          Add a custom server
        </button>
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
