"use client";

import { useState, useEffect, useCallback, Suspense } from "react";
import { useSearchParams, useRouter } from "next/navigation";
import {
  ArrowLeftIcon,
  ChatBubbleLeftRightIcon,
  PlayIcon,
  StopIcon,
  TrashIcon,
  InformationCircleIcon,
  Cog6ToothIcon,
  KeyIcon,
} from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";
import { sseBus } from "@/lib/sse-event-bus";
import { SectionHeader, SectionPanel } from "@/components/settings/field";
import { AddCredentialForm } from "@/components/agents/configure/creds-section";
import type { VaultGrant, VaultConnection, PendingCredential } from "@/components/agents/configure/creds-section";
import { formatDistanceToNow } from "date-fns";
import type { Agent, SpaceResponse } from "@/lib/types";

interface UserAddress {
  address: string | null;
  pairing_code: string | null;
  pairing_initiated_at: string | null;
  paired_at: string | null;
}

interface Channel {
  id: string;
  user_id: string;
  space_id: string;
  provider: string;
  agent_id: string;
  config: Record<string, string>;
  dispatch_mode: "message" | "signal";
  status: "disconnected" | "connecting" | "connected" | "failed" | "pairing" | "setup";
  error_message: string | null;
  last_started_at: string | null;
  user_address: UserAddress | null;
  created_at: string;
  updated_at: string;
}

interface ChannelConfigField {
  name: string;
  description: string | null;
  is_required: boolean;
  is_secret: boolean;
  format: string | null;
  // For secret fields the resolved value is stripped server-side; presence of
  // `default_from` is the only signal that a server-config fallback exists.
  default_from: { section: string; field: string } | null;
  default_resolved: string | null;
}

const DISPATCH_MODE_LABEL: Record<string, string> = {
  message: "Process as your message",
  signal: "Hand off to a waiting agent",
};

interface ChannelManifest {
  id: string;
  display_name: string;
  description: string;
  config_fields: ChannelConfigField[];
}

const SECTIONS = [
  { id: "status", label: "Status" },
  { id: "config", label: "Config" },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const STATUS_BADGE: Record<string, string> = {
  disconnected: "bg-surface-tertiary text-text-secondary",
  connecting: "bg-blue-400/15 text-blue-400",
  connected: "bg-green-500/15 text-green-500",
  failed: "bg-red-500/15 text-red-500",
  pairing: "bg-purple-500/15 text-purple-400",
  setup: "bg-yellow-500/15 text-yellow-500",
};

// Green is reserved for the `connected` status badge — keep it out of the
// provider palette so the two don't collide visually on a row.
const PROVIDER_COLORS = [
  "bg-blue-500/15 text-blue-400",
  "bg-indigo-500/15 text-indigo-400",
  "bg-purple-500/15 text-purple-400",
  "bg-pink-500/15 text-pink-400",
  "bg-orange-500/15 text-orange-400",
  "bg-cyan-500/15 text-cyan-400",
];

function providerBadgeClass(providerId: string, manifests: ChannelManifest[]): string {
  const idx = manifests.findIndex((m) => m.id === providerId);
  if (idx < 0) return "bg-surface-tertiary text-text-tertiary";
  return PROVIDER_COLORS[idx % PROVIDER_COLORS.length];
}

function ChannelDetailPage() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const channelId = searchParams.get("id");

  const [channel, setChannel] = useState<Channel | null>(null);
  const [manifest, setManifest] = useState<ChannelManifest | null>(null);
  const [allManifests, setAllManifests] = useState<ChannelManifest[]>([]);
  const [space, setSpace] = useState<SpaceResponse | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);

  const [agentId, setAgentId] = useState("");
  const [dispatchMode, setDispatchMode] = useState<"message" | "signal">("message");
  const [config, setConfig] = useState<Record<string, string>>({});

  // Picker writes are staged locally; the page Save button commits pending
  // grants and deleted grants alongside config changes so the swap is atomic.
  const [connections, setConnections] = useState<Map<string, VaultConnection>>(new Map());
  const [grants, setGrants] = useState<VaultGrant[]>([]);
  const [pendingCreds, setPendingCreds] = useState<PendingCredential[]>([]);
  const [deletedGrantIds, setDeletedGrantIds] = useState<Set<string>>(new Set());
  const [credDialogEnvVar, setCredDialogEnvVar] = useState<string | null>(null);
  const [credDialogExisting, setCredDialogExisting] = useState<
    { connection_id: string; vault_item_id: string } | undefined
  >(undefined);

  const sectionParam = searchParams.get("section");
  const initialSection = SECTIONS.some((s) => s.id === sectionParam)
    ? (sectionParam as SectionId)
    : "status";
  const [activeSection, setActiveSectionState] = useState<SectionId>(initialSection);

  const setActiveSection = useCallback(
    (section: SectionId) => {
      setActiveSectionState(section);
      const params = new URLSearchParams(searchParams.toString());
      params.set("section", section);
      router.replace(`/channels?${params.toString()}`, { scroll: false });
    },
    [router, searchParams],
  );

  const reload = useCallback(async () => {
    if (!channelId) return;
    try {
      const [c, mans, ags, conns, allGrants, spcs] = await Promise.all([
        api.get<Channel>(`/api/channels/${channelId}`),
        api.get<ChannelManifest[]>("/api/channels/manifests"),
        api.get<Agent[]>("/api/agents"),
        api.get<VaultConnection[]>("/api/vaults"),
        api.get<VaultGrant[]>("/api/vaults/grants"),
        api.get<SpaceResponse[]>("/api/spaces"),
      ]);
      setChannel(c);
      setManifest(mans.find((m) => m.id === c.provider) ?? null);
      setAllManifests(mans);
      setSpace(spcs.find((s) => s.id === c.space_id) ?? null);
      setAgents(ags);
      setAgentId(c.agent_id);
      setDispatchMode(c.dispatch_mode);
      setConfig(c.config ?? {});
      setConnections(new Map(conns.map((cn) => [cn.id, cn])));
      setGrants(
        allGrants.filter(
          (g) => g.principal?.kind === "channel" && g.principal?.id === channelId,
        ),
      );
      setPendingCreds([]);
      setDeletedGrantIds(new Set());
      setDirty(false);
    } catch {
      setError("Failed to load channel");
    } finally {
      setLoading(false);
    }
  }, [channelId]);

  useEffect(() => {
    reload();
  }, [reload]);

  const save = async () => {
    if (!channel) return;
    setSaving(true);
    setError(null);
    try {
      // Apply credential changes before the config PATCH — the PATCH triggers
      // a channel restart that must observe the final binding set.
      for (const id of deletedGrantIds) {
        await api.delete(`/api/vaults/grants/${id}`);
      }
      for (const cred of pendingCreds) {
        await api.post("/api/vaults/grants", cred);
      }

      const patch: Record<string, unknown> = {};
      if (agentId !== channel.agent_id) patch.agent_id = agentId;
      if (dispatchMode !== channel.dispatch_mode) patch.dispatch_mode = dispatchMode;
      const cfgChanged = JSON.stringify(config) !== JSON.stringify(channel.config ?? {});
      if (cfgChanged) patch.config = config;
      if (Object.keys(patch).length > 0) {
        await api.patch(`/api/channels/${channelId}`, patch);
      }
      await reload();
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
      await api.post(`/api/channels/${channelId}/start`, {});
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Start failed");
    } finally {
      await reload();
      setActionLoading(false);
    }
  };

  const stop = async () => {
    setActionLoading(true);
    try {
      await api.post(`/api/channels/${channelId}/stop`, {});
      await reload();
    } catch {
    } finally {
      setActionLoading(false);
    }
  };

  const remove = async () => {
    if (!confirm("Delete this channel? Pending messages will not be sent.")) return;
    setActionLoading(true);
    try {
      await api.delete(`/api/channels/${channelId}`);
      router.push("/settings#channels");
    } catch {
    } finally {
      setActionLoading(false);
    }
  };

  const initiatePairing = async () => {
    setActionLoading(true);
    setError(null);
    try {
      await api.post(`/api/channels/${channelId}/pair`, {});
      await reload();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Pairing failed");
    } finally {
      setActionLoading(false);
    }
  };

  const cancelPairing = async () => {
    setActionLoading(true);
    try {
      await api.delete(`/api/channels/${channelId}/pair`);
      await reload();
    } catch {
    } finally {
      setActionLoading(false);
    }
  };

  useEffect(() => {
    if (!channelId) return;
    const unsub = sseBus.onGlobal((event) => {
      if (event.type === "entity_updated"
          && event.table === "channel"
          && event.recordId === channelId) {
        reload();
      }
    });
    return unsub;
  }, [channelId, reload]);

  if (!channelId) {
    return <p className="p-8 text-sm text-error-text">No channel ID provided.</p>;
  }
  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent" />
      </div>
    );
  }
  if (!channel) {
    return <p className="p-8 text-sm text-error-text">{error || "Channel not found"}</p>;
  }

  const canStart = channel.status === "disconnected" || channel.status === "failed";
  const canStop = channel.status === "connected" || channel.status === "connecting";
  const displayName = space?.name ?? manifest?.display_name ?? channel.provider;

  return (
    <div className="flex h-full bg-surface">
      <div
        className="border-r border-border bg-surface-nav p-4 flex flex-col"
        style={{ width: 289 }}
      >
        <button
          onClick={() => router.push("/settings#channels")}
          className="flex items-center gap-2 text-sm text-text-secondary hover:text-text-primary transition mb-4"
        >
          <ArrowLeftIcon className="h-4 w-4" />
          Back to Channels
        </button>

        <div className="flex items-center gap-2 mb-4">
          <ChatBubbleLeftRightIcon className="h-8 w-8 text-text-tertiary shrink-0" />
          <div className="min-w-0">
            <h2 className="text-sm font-semibold text-text-primary truncate">{displayName}</h2>
            <div className="flex items-center gap-1 flex-wrap mt-0.5">
              <span
                className={`inline-block rounded-full px-2 py-0.5 text-[10px] font-medium ${providerBadgeClass(channel.provider, allManifests)}`}
              >
                {manifest?.display_name ?? channel.provider}
              </span>
              <span
                className={`inline-block rounded-full px-2 py-0.5 text-[10px] font-medium ${
                  STATUS_BADGE[channel.status] ?? STATUS_BADGE.disconnected
                }`}
              >
                {channel.status}
              </span>
            </div>
          </div>
        </div>

        <nav className="space-y-1 flex-1">
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              onClick={() => setActiveSection(s.id)}
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
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-2xl mx-auto p-8 space-y-6">
          {activeSection === "status" && (
            <div className="space-y-6">
              <SectionHeader
                title="Status"
                description="Connection state and lifecycle controls"
                icon={InformationCircleIcon}
              />
              {error && (
                <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-400">
                  {error}
                </div>
              )}
              <div className="rounded-xl border border-border bg-surface-secondary divide-y divide-border overflow-hidden">
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Provider</span>
                  <span className="text-sm text-text-primary font-mono">{channel.provider}</span>
                </div>
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Status</span>
                  <span
                    className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${
                      STATUS_BADGE[channel.status] ?? STATUS_BADGE.disconnected
                    }`}
                  >
                    {channel.status}
                  </span>
                </div>
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Dispatch mode</span>
                  <span className="text-sm text-text-primary">
                    {DISPATCH_MODE_LABEL[channel.dispatch_mode] ?? channel.dispatch_mode}
                  </span>
                </div>
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Agent</span>
                  <span className="text-sm text-text-primary">
                    {agents.find((a) => a.id === channel.agent_id)?.name ?? channel.agent_id}
                  </span>
                </div>
                <div className="px-4 py-3 flex justify-between">
                  <span className="text-sm text-text-tertiary">Created</span>
                  <span className="text-sm text-text-primary">
                    {formatDistanceToNow(new Date(channel.created_at), { addSuffix: true })}
                  </span>
                </div>
                {channel.last_started_at && (
                  <div className="px-4 py-3 flex justify-between">
                    <span className="text-sm text-text-tertiary">Last started</span>
                    <span className="text-sm text-text-primary">
                      {formatDistanceToNow(new Date(channel.last_started_at), { addSuffix: true })}
                    </span>
                  </div>
                )}
                {channel.error_message && (
                  <div className="px-4 py-3 flex justify-between gap-4">
                    <span className="text-sm text-text-tertiary shrink-0">Last error</span>
                    <span className="text-sm text-red-400 text-right break-words">
                      {channel.error_message}
                    </span>
                  </div>
                )}
              </div>

              {channel.status === "pairing" && channel.user_address?.pairing_code && (
                <div className="rounded-xl border border-purple-500/30 bg-purple-500/5 p-4 space-y-3">
                  <div>
                    <h4 className="text-sm font-semibold text-text-primary">
                      Awaiting pairing
                    </h4>
                    <p className="text-xs text-text-tertiary mt-1">
                      Send the code below as a message to this channel from the
                      account you want to pair. The channel will flip to Connected
                      automatically.
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 rounded-lg border border-border bg-surface px-3 py-3 text-center text-2xl font-mono font-semibold tracking-[0.3em] text-text-primary">
                      {channel.user_address.pairing_code}
                    </code>
                    <button
                      onClick={() => navigator.clipboard.writeText(channel.user_address!.pairing_code!)}
                      className="rounded-lg border border-border px-3 py-2 text-xs text-text-secondary hover:bg-surface-tertiary transition"
                    >
                      Copy
                    </button>
                  </div>
                  <button
                    onClick={cancelPairing}
                    disabled={actionLoading}
                    className="text-xs text-text-tertiary hover:text-danger transition"
                  >
                    Cancel pairing
                  </button>
                </div>
              )}

              {channel.user_address?.address && channel.status !== "pairing" && (
                <div className="rounded-xl border border-border bg-surface-secondary px-4 py-3 flex items-center justify-between">
                  <div>
                    <span className="text-xs text-text-tertiary">Paired as</span>
                    <div className="text-sm font-mono text-text-primary mt-0.5">
                      {channel.user_address.address}
                    </div>
                  </div>
                  <button
                    onClick={initiatePairing}
                    disabled={actionLoading}
                    className="text-xs text-accent hover:text-accent-hover transition"
                  >
                    Re-pair
                  </button>
                </div>
              )}

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
                {channel.status !== "pairing" && !channel.user_address?.address && (
                  <button
                    onClick={initiatePairing}
                    disabled={actionLoading}
                    className="inline-flex items-center gap-1.5 rounded-lg bg-purple-600 px-4 py-2 text-sm font-medium text-white hover:bg-purple-700 disabled:opacity-50 transition"
                  >
                    {actionLoading ? "Generating..." : "Pair"}
                  </button>
                )}
                <button
                  onClick={remove}
                  disabled={actionLoading}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-border px-4 py-2 text-sm font-medium text-danger hover:bg-surface-tertiary disabled:opacity-50 transition"
                >
                  <TrashIcon className="h-4 w-4" />
                  Delete
                </button>
              </div>
            </div>
          )}

          {activeSection === "config" && (
            <div className="space-y-4">
              <SectionHeader
                title="Config"
                description="Provider-specific settings, agent binding, and dispatch mode"
                icon={Cog6ToothIcon}
              />

              <SectionPanel title="Routing">
                <div className="space-y-3">
                  <div className="space-y-1">
                    <label className="text-xs font-medium text-text-secondary">Agent</label>
                    <select
                      value={agentId}
                      onChange={(e) => {
                        setAgentId(e.target.value);
                        setDirty(true);
                      }}
                      className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
                    >
                      {agents.map((a) => (
                        <option key={a.id} value={a.id}>
                          {a.name}
                        </option>
                      ))}
                    </select>
                  </div>

                  <div className="space-y-1">
                    <label className="text-xs font-medium text-text-secondary">
                      When a message arrives
                    </label>
                    <select
                      value={dispatchMode}
                      onChange={(e) => {
                        setDispatchMode(e.target.value as "message" | "signal");
                        setDirty(true);
                      }}
                      className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
                    >
                      <option value="message">Treat as a message from you — requires pairing</option>
                      <option value="signal">Hand off to a waiting agent — e.g. 2FA codes or confirmation links</option>
                    </select>
                  </div>
                </div>
              </SectionPanel>

              {manifest && manifest.config_fields.length > 0 && (
                <SectionPanel title="Provider Config">
                  <div className="space-y-3">
                    {manifest.config_fields.map((f) => {
                      const liveGrant = grants
                        .filter((g) => !deletedGrantIds.has(g.id))
                        .find((g) => g.query === f.name);
                      const pending = pendingCreds.find((p) => p.query === f.name);
                      return (
                        <div key={f.name} className="space-y-1">
                          <div className="flex items-center gap-2">
                            <label className="text-xs font-medium text-text-secondary">
                              {f.description || f.name}
                            </label>
                            {f.is_required && (
                              <span className="text-[10px] text-red-400">required</span>
                            )}
                          </div>
                          {f.is_secret ? (
                            liveGrant ? (
                              <div className="flex items-center gap-2">
                                <KeyIcon className="h-4 w-4 text-text-tertiary shrink-0" />
                                <span className="text-sm text-text-primary truncate">
                                  Configured
                                  <span className="text-text-tertiary"> ({connections.get(liveGrant.connection_id)?.name ?? "vault"})</span>
                                </span>
                                <button
                                  type="button"
                                  onClick={() => {
                                    setCredDialogExisting({
                                      connection_id: liveGrant.connection_id,
                                      vault_item_id: liveGrant.vault_item_id,
                                    });
                                    setCredDialogEnvVar(f.name);
                                  }}
                                  className="text-xs text-accent hover:underline"
                                >
                                  Change
                                </button>
                                <button
                                  type="button"
                                  onClick={() => {
                                    setDeletedGrantIds((prev) => new Set(prev).add(liveGrant.id));
                                    setDirty(true);
                                  }}
                                  className="text-xs text-danger hover:underline"
                                >
                                  Remove
                                </button>
                              </div>
                            ) : pending ? (
                              <div className="flex items-center gap-2">
                                <KeyIcon className="h-4 w-4 text-text-tertiary shrink-0" />
                                <span className="text-sm text-text-primary truncate">
                                  {pending.item_name}
                                  <span className="text-text-tertiary"> ({pending.connection_name})</span>
                                </span>
                                <span className="rounded-full bg-yellow-500/15 px-2 py-0.5 text-[10px] font-medium text-yellow-500">
                                  Pending save
                                </span>
                                <button
                                  type="button"
                                  onClick={() => {
                                    setPendingCreds((prev) => prev.filter((p) => p.query !== f.name));
                                    setDirty(true);
                                  }}
                                  className="text-xs text-danger hover:underline"
                                >
                                  Discard
                                </button>
                              </div>
                            ) : f.default_from ? (
                              <div className="flex items-center gap-2">
                                <KeyIcon className="h-4 w-4 text-text-tertiary shrink-0" />
                                <span className="text-sm text-text-primary truncate">
                                  Using server default
                                  <span className="text-text-tertiary"> ({f.default_from.section}.{f.default_from.field})</span>
                                </span>
                                <button
                                  type="button"
                                  onClick={() => {
                                    setCredDialogExisting(undefined);
                                    setCredDialogEnvVar(f.name);
                                  }}
                                  className="text-xs text-accent hover:underline"
                                >
                                  Override
                                </button>
                              </div>
                            ) : (
                              <button
                                type="button"
                                onClick={() => {
                                  setCredDialogExisting(undefined);
                                  setCredDialogEnvVar(f.name);
                                }}
                                className="text-sm transition cursor-pointer"
                              >
                                <span className="text-text-tertiary">Not set — </span>
                                <span className="text-accent hover:underline">Configure</span>
                              </button>
                            )
                          ) : (
                            <input
                              type="text"
                              value={config[f.name] ?? ""}
                              onChange={(e) => {
                                setConfig((prev) => ({ ...prev, [f.name]: e.target.value }));
                                setDirty(true);
                              }}
                              placeholder={
                                f.default_resolved
                                  ? `default: ${f.default_resolved}`
                                  : f.is_required
                                    ? "Required"
                                    : "Optional"
                              }
                              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none font-mono"
                            />
                          )}
                        </div>
                      );
                    })}
                  </div>
                </SectionPanel>
              )}
            </div>
          )}

          {credDialogEnvVar !== null && channel && (
            <AddCredentialForm
              connections={connections}
              principalKind="channel"
              principalId={channel.id}
              existingGrants={grants}
              targetEnvVar={credDialogEnvVar}
              initialSelection={credDialogExisting}
              onClose={() => {
                setCredDialogEnvVar(null);
                setCredDialogExisting(undefined);
              }}
              // For "Change", stage the removal of the existing grant alongside
              // the new pending credential so Save commits the swap atomically.
              deferred={(pending) => {
                if (credDialogExisting) {
                  const replacing = grants.find(
                    (g) =>
                      g.connection_id === credDialogExisting.connection_id
                      && g.vault_item_id === credDialogExisting.vault_item_id,
                  );
                  if (replacing) {
                    setDeletedGrantIds((prev) => new Set(prev).add(replacing.id));
                  }
                }
                setPendingCreds((prev) => [
                  ...prev.filter((p) => p.query !== pending.query),
                  pending,
                ]);
                setDirty(true);
                setCredDialogEnvVar(null);
                setCredDialogExisting(undefined);
              }}
              onCreated={() => {
                setCredDialogEnvVar(null);
                setCredDialogExisting(undefined);
              }}
            />
          )}

          {activeSection !== "status" && (
            <div className="pt-4 border-t border-border flex items-center justify-end gap-2">
              <button
                onClick={() => {
                  reload();
                }}
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
    </div>
  );
}

export default function ChannelsPage() {
  return (
    <Suspense>
      <ChannelDetailPage />
    </Suspense>
  );
}
