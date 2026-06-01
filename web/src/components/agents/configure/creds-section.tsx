"use client";

import { useState, useEffect, useCallback, useRef } from "react";
import { SectionHeader } from "@/components/settings/field";
import { KeyIcon, TrashIcon, PlusIcon, MagnifyingGlassIcon } from "@heroicons/react/24/outline";
import { CheckIcon, MinusIcon } from "@heroicons/react/16/solid";
import * as Checkbox from "@radix-ui/react-checkbox";
import { Si1password, SiBitwarden, SiVault, SiKeepassxc } from "@icons-pack/react-simple-icons";
import { api } from "@/lib/api-client";

const PROVIDER_ICONS: Record<string, React.ComponentType<{ className?: string; size?: number }>> = {
  one_password: Si1password,
  bitwarden: SiBitwarden,
  hashicorp: SiVault,
  keepass: SiKeepassxc,
};

const PROVIDER_LABELS: Record<string, string> = {
  local: "Local",
  one_password: "1Password",
  bitwarden: "Bitwarden",
  hashicorp: "HashiCorp",
  keepass: "KeePass",
};

export interface VaultGrant {
  id: string;
  connection_id: string;
  vault_item_id: string;
  principal: { kind: string; id: string };
  query: string;
  target?: { Single?: { env_var: string; field: unknown }; Prefix?: { env_var_prefix: string } };
  expires_at: string | null;
  created_at: string;
}

export interface VaultConnection {
  id: string;
  name: string;
  provider: string;
  enabled: boolean;
}

interface VaultItem {
  id: string;
  name: string;
  username: string | null;
}

export interface CredsSectionProps {
  principalKind: "agent" | "mcp_server" | "app";
  principalId: string;
}

function matchesPrincipal(grant: VaultGrant, kind: string, id: string): boolean {
  return grant.principal?.kind === kind && grant.principal?.id === id;
}

function EnvPreview({ prefix, fields }: { prefix: string; fields: string[] }) {
  if (fields.length === 0) return null;
  const sep = prefix ? "_" : "";
  return (
    <div className="mt-2 space-y-1.5">
      <p className="text-[11px] text-text-tertiary">
        The following environment variables will be available at runtime:
      </p>
      <div className="flex flex-wrap gap-1.5">
        {fields.map((f) => (
          <span key={f} className="rounded-full border border-border bg-surface-tertiary px-2.5 py-0.5 font-mono text-xs text-text-primary">
            {prefix}{sep}{f}
          </span>
        ))}
      </div>
    </div>
  );
}

export interface PendingCredential {
  principal: { kind: string; id: string };
  connection_id: string;
  vault_item_id: string;
  query: string;
  target: unknown;
  item_name: string;
  connection_name: string;
  fields: string[];
}

export function AddCredentialForm({
  connections,
  principalKind,
  principalId,
  existingGrants,
  targetEnvVar,
  initialSelection,
  onClose,
  onCreated,
  deferred,
}: {
  connections: Map<string, VaultConnection>;
  principalKind: string;
  principalId: string;
  existingGrants: VaultGrant[];
  targetEnvVar?: string;
  initialSelection?: { connection_id: string; vault_item_id: string };
  onClose: () => void;
  onCreated: (grant: VaultGrant) => void;
  deferred?: (pending: PendingCredential) => void;
}) {
  const enabledConns = Array.from(connections.values()).filter((c) => c.enabled);
  const [selectedConnection, setSelectedConnection] = useState(initialSelection?.connection_id ?? enabledConns[0]?.id ?? "");
  const [items, setItems] = useState<VaultItem[]>([]);
  const [selectedItem, setSelectedItem] = useState(initialSelection?.vault_item_id ?? "");
  const [searchQuery, setSearchQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [envVar, setEnvVar] = useState("");
  const [envVarManuallyEdited, setEnvVarManuallyEdited] = useState(false);
  const [bindingMode, setBindingMode] = useState<"prefix" | "all" | "single">("prefix");
  const [fields, setFields] = useState<string[]>([]);
  const [loadingFields, setLoadingFields] = useState(false);
  const [selectedField, setSelectedField] = useState("");
  const [creating, setCreating] = useState(false);
  const [newType, setNewType] = useState<"ApiKey" | "UsernamePassword">("ApiKey");
  const [newName, setNewName] = useState("");
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [newApiKey, setNewApiKey] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  // Latest predicate values consumed by the search effect — captured in a ref
  // so the search doesn't refire when only these guards change.
  const searchGuardsRef = useRef({ selectedItem, envVarManuallyEdited, bindingMode });
  searchGuardsRef.current = { selectedItem, envVarManuallyEdited, bindingMode };

  useEffect(() => {
    if (!selectedConnection) return;
    const controller = new AbortController();
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(async () => {
      setSearching(true);
      try {
        const results = await api.get<VaultItem[]>(`/api/vaults/${selectedConnection}/items?q=${encodeURIComponent(searchQuery)}`);
        if (!controller.signal.aborted) {
          setItems(results);
          const guards = searchGuardsRef.current;
          if (results.length > 0 && !guards.selectedItem) {
            const first = results[0];
            setSelectedItem(first.id);
            if (!guards.envVarManuallyEdited) setEnvVar(first.name.toUpperCase().replace(/[^A-Z0-9]/g, "_"));
            setLoadingFields(true);
            api.get<string[]>(`/api/vaults/${selectedConnection}/items/${first.id}/fields`)
              .then((f) => {
                setFields(f);
                if (f.length > 0) {
                  setSelectedField(f[0]);
                  if (!guards.envVarManuallyEdited && guards.bindingMode === "single") {
                    setEnvVar(`${first.name.toUpperCase().replace(/[^A-Z0-9]/g, "_")}_${f[0]}`);
                  }
                }
              })
              .catch(() => setFields([]))
              .finally(() => setLoadingFields(false));
          }
        }
      } catch {
        if (!controller.signal.aborted) setItems([]);
      } finally {
        if (!controller.signal.aborted) setSearching(false);
      }
    }, 300);
    return () => { controller.abort(); if (debounceRef.current) clearTimeout(debounceRef.current); };
  }, [selectedConnection, searchQuery]);

  const initialVaultItemId = initialSelection?.vault_item_id;
  const didLoadInitialFieldsRef = useRef(false);
  useEffect(() => {
    if (didLoadInitialFieldsRef.current) return;
    if (initialVaultItemId && selectedConnection) {
      didLoadInitialFieldsRef.current = true;
      api.get<string[]>(`/api/vaults/${selectedConnection}/items/${initialVaultItemId}/fields`)
        .then((f) => { setFields(f); if (f.length > 0) setSelectedField(f[0]); })
        .catch(() => setFields([]));
    }
  }, [initialVaultItemId, selectedConnection]);

  const createLocalItem = async () => {
    if (!newName.trim()) return;
    const body = newType === "ApiKey"
      ? { type: "ApiKey", name: newName.trim(), api_key: newApiKey.trim() }
      : { type: "UsernamePassword", name: newName.trim(), username: newUsername.trim(), password: newPassword.trim() };
    setSaving(true);
    setError(null);
    try {
      const created = await api.post<{ id: string; name: string }>("/api/vaults/local/items", body);
      // Auto-select local vault if not already
      const localConn = Array.from(connections.values()).find((c) => c.provider === "local");
      if (localConn && selectedConnection !== localConn.id) setSelectedConnection(localConn.id);
      setSelectedItem(created.id);
      if (!envVar) setEnvVar(newName.trim().toUpperCase().replace(/[^A-Z0-9]/g, "_"));
      setCreating(false);
      setNewName(""); setNewUsername(""); setNewPassword(""); setNewApiKey("");
      const connId = localConn?.id ?? selectedConnection;
      const results = await api.get<VaultItem[]>(`/api/vaults/${connId}/items?q=${encodeURIComponent(searchQuery)}`);
      setItems(results);
      api.get<string[]>(`/api/vaults/${connId}/items/${created.id}/fields`)
        .then((f) => { setFields(f); if (f.length > 0) setSelectedField(f[0]); }).catch(() => setFields([]));
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to create credential");
    } finally {
      setSaving(false);
    }
  };

  const submit = async () => {
    if (!selectedConnection || !selectedItem) return;
    setSaving(true);
    setError(null);
    try {
      const mode = targetEnvVar ? "single" : bindingMode;
      const query = targetEnvVar ?? envVar.trim();
      const toVaultField = (f: string) => {
        if (f === "PASSWORD") return "Password";
        if (f === "USERNAME") return "Username";
        return { Custom: { name: f } };
      };
      const target = mode === "single"
        ? { Single: { env_var: query, field: toVaultField(selectedField || "PASSWORD") } }
        : { Prefix: { env_var_prefix: mode === "prefix" ? envVar.trim() : "" } };
      const payload = {
        principal: { kind: principalKind, id: principalId },
        connection_id: selectedConnection,
        vault_item_id: selectedItem,
        query,
        target,
      };
      if (deferred) {
        const item = items.find((i) => i.id === selectedItem);
        const conn = connections.get(selectedConnection);
        deferred({
          ...payload,
          item_name: item?.name ?? query,
          connection_name: conn?.name ?? "Unknown",
          fields: [...fields],
        });
        onClose();
        return;
      }
      const grant = await api.post<VaultGrant>("/api/vaults/grants", payload);
      onCreated(grant);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to create grant");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      <div className="relative w-full max-w-lg rounded-xl border border-border bg-surface-secondary p-5 shadow-xl mx-4 space-y-3">
      <SectionHeader
        title={creating ? "New credential" : "Add credential"}
        description={creating
          ? "Create a new credential in the local vault"
          : targetEnvVar
            ? `Select a credential for ${targetEnvVar}`
            : "Select a credential from your vault"}
        icon={KeyIcon}
      />

      {creating ? (
        <div className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-text-tertiary mb-1">Type</label>
            <select
              value={newType}
              onChange={(e) => setNewType(e.target.value as "ApiKey" | "UsernamePassword")}
              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
            >
              <option value="ApiKey">API Key</option>
              <option value="UsernamePassword">Username & Password</option>
            </select>
          </div>
          <div>
            <label className="block text-xs font-medium text-text-tertiary mb-1">Name</label>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="e.g. Google OAuth"
              autoFocus
              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
            />
          </div>
          {newType === "UsernamePassword" ? (
            <>
              <div>
                <label className="block text-xs font-medium text-text-tertiary mb-1">Username</label>
                <input
                  type="text"
                  value={newUsername}
                  onChange={(e) => setNewUsername(e.target.value)}
                  placeholder="Username"
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-text-tertiary mb-1">Password</label>
                <input
                  type="password"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  placeholder="Password"
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
                />
              </div>
            </>
          ) : (
            <div>
              <label className="block text-xs font-medium text-text-tertiary mb-1">API Key</label>
              <input
                type="password"
                value={newApiKey}
                onChange={(e) => setNewApiKey(e.target.value)}
                placeholder="API Key"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
              />
            </div>
          )}
          <EnvPreview
            prefix={newName.trim().toUpperCase().replace(/[^A-Z0-9]/g, "_")}
            fields={newType === "ApiKey" ? ["API_KEY"] : ["USERNAME", "PASSWORD"]}
          />
          {error && <p className="text-xs text-danger">{error}</p>}
          <div className="flex items-center gap-2">
            <div className="flex-1" />
            <button
              onClick={() => { setCreating(false); setNewName(""); setNewUsername(""); setNewPassword(""); setNewApiKey(""); setError(null); }}
              className="w-24 inline-flex items-center justify-center rounded-lg border border-border py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
            >
              Back
            </button>
            <button
              onClick={createLocalItem}
              disabled={!newName.trim() || (newType === "ApiKey" ? !newApiKey.trim() : !newUsername.trim() || !newPassword.trim()) || saving}
              className="w-24 inline-flex items-center justify-center rounded-lg bg-accent py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
            >
              {saving ? "Creating..." : "Create"}
            </button>
          </div>
        </div>
      ) : (<>

      {/* Vault selector */}
      <div>
        <label className="block text-xs font-medium text-text-tertiary mb-1">Vault</label>
        <select
          value={selectedConnection}
          onChange={(e) => { setSelectedConnection(e.target.value); setItems([]); setSelectedItem(""); }}
          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
        >
          {enabledConns.map((c) => (
            <option key={c.id} value={c.id}>{c.name}</option>
          ))}
        </select>
      </div>

      {/* Search */}
      <div>
        <label className="block text-xs font-medium text-text-tertiary mb-1">Search</label>
        <div className="relative">
          <MagnifyingGlassIcon className="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-text-tertiary" />
          <input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search vault items..."
            className="w-full rounded-lg border border-border bg-surface pl-8 pr-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
          />
        </div>
      </div>

      {/* Items */}
      <div>
        <label className="block text-xs font-medium text-text-tertiary mb-1">Item</label>
        {searching ? (
          <p className="text-xs text-text-tertiary py-1">Searching...</p>
        ) : items.length > 0 ? (
          <div className="space-y-1 max-h-40 overflow-y-auto rounded-lg border border-border p-1">
            {items.map((item) => {
              const alreadyGranted = !targetEnvVar && existingGrants.some(
                (g) => g.connection_id === selectedConnection && g.vault_item_id === item.id
              );
              return (
                <button
                  key={item.id}
                  disabled={alreadyGranted}
                  onClick={() => {
                    setSelectedItem(item.id);
                    const itemPrefix = item.name.toUpperCase().replace(/[^A-Z0-9]/g, "_");
                    if (!envVarManuallyEdited) setEnvVar(itemPrefix);
                    setFields([]);
                    setSelectedField("");
                    setLoadingFields(true);
                    api.get<string[]>(`/api/vaults/${selectedConnection}/items/${item.id}/fields`)
                      .then((f) => {
                        setFields(f);
                        if (f.length > 0) {
                          setSelectedField(f[0]);
                          if (!envVarManuallyEdited && bindingMode === "single") {
                            setEnvVar(`${itemPrefix}_${f[0]}`);
                          }
                        }
                      }).catch(() => setFields([]))
                      .finally(() => setLoadingFields(false));
                  }}
                  className={`w-full rounded-lg border px-3 py-2 text-left text-sm transition ${
                    alreadyGranted
                      ? "border-border text-text-tertiary opacity-50 cursor-not-allowed"
                      : selectedItem === item.id
                        ? "border-accent bg-accent/10 text-accent"
                        : "border-border text-text-secondary hover:border-accent"
                  }`}
                >
                  <span className="font-medium">{item.name}</span>
                  {item.username && <span className="ml-2 text-text-tertiary">({item.username})</span>}
                  {alreadyGranted && <span className="ml-2 text-[10px] text-text-tertiary">already assigned</span>}
                </button>
              );
            })}
          </div>
        ) : (
          <p className="text-xs text-text-tertiary py-1">No items found</p>
        )}
      </div>

      {/* Target configuration */}
      {targetEnvVar ? (
        loadingFields ? (
          <p className="text-xs text-text-tertiary py-1">Loading fields...</p>
        ) : fields.length > 0 ? (
          <div>
            <label className="block text-xs font-medium text-text-tertiary mb-1">Field</label>
            <div className="flex flex-wrap gap-1.5">
              {fields.map((f) => (
                <button
                  key={f}
                  type="button"
                  onClick={() => setSelectedField(f)}
                  className={`rounded-full px-3.5 py-1.5 text-sm transition ${
                    selectedField === f
                      ? "bg-accent text-surface"
                      : "bg-surface-tertiary text-text-secondary hover:text-text-primary"
                  }`}
                >
                  {f.toLowerCase()}
                </button>
              ))}
            </div>
          </div>
        ) : null
      ) : (
        <div className="space-y-2">
          <label className="block text-xs font-medium text-text-tertiary mb-1">Binding mode</label>
          <div className="flex flex-wrap gap-1.5">
            {([
              { value: "prefix" as const, label: "With prefix" },
              { value: "all" as const, label: "All fields" },
              { value: "single" as const, label: "Single field" },
            ]).map((opt) => (
              <button
                key={opt.value}
                type="button"
                onClick={() => {
                  setBindingMode(opt.value);
                  setEnvVarManuallyEdited(false);
                  if (opt.value === "single" && selectedItem && selectedField) {
                    const itemName = items.find((i) => i.id === selectedItem)?.name ?? "";
                    setEnvVar(`${itemName}_${selectedField}`.toUpperCase().replace(/[^A-Z0-9_]/g, "_"));
                  } else if (opt.value === "prefix" && selectedItem) {
                    const itemName = items.find((i) => i.id === selectedItem)?.name ?? "";
                    setEnvVar(itemName.toUpperCase().replace(/[^A-Z0-9]/g, "_"));
                  } else if (opt.value === "all") {
                    setEnvVar("");
                  }
                }}
                className={`rounded-full px-3.5 py-1.5 text-sm transition ${
                  bindingMode === opt.value
                    ? "bg-accent text-surface"
                    : "bg-surface-tertiary text-text-secondary hover:text-text-primary"
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>

          {bindingMode === "prefix" && (
            <div>
              <input
                type="text"
                value={envVar}
                onChange={(e) => { setEnvVar(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, "")); setEnvVarManuallyEdited(true); }}
                placeholder="e.g. GOOGLE_OAUTH"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none font-mono"
              />
              <EnvPreview prefix={envVar} fields={fields} />
            </div>
          )}

          {bindingMode === "all" && (
            loadingFields
              ? <p className="text-xs text-text-tertiary py-1">Loading fields...</p>
              : <EnvPreview prefix="" fields={fields} />
          )}

          {bindingMode === "single" && (
            <div className="space-y-2">
              <div>
                <label className="block text-xs font-medium text-text-tertiary mb-1">Environment variable</label>
                <input
                  type="text"
                  value={envVar}
                  onChange={(e) => { setEnvVar(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, "")); setEnvVarManuallyEdited(true); }}
                  placeholder="ENV_VAR_NAME"
                  className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none font-mono"
                />
              </div>
              {loadingFields && (
                <p className="text-xs text-text-tertiary py-1">Loading fields...</p>
              )}
              {!loadingFields && fields.length > 0 && (
                <div>
                  <label className="block text-xs font-medium text-text-tertiary mb-1">Field</label>
                  <div className="flex flex-wrap gap-1.5">
                    {fields.map((f) => (
                      <button
                        key={f}
                        type="button"
                        onClick={() => {
                          setSelectedField(f);
                          if (!envVarManuallyEdited) {
                            const itemName = items.find((i) => i.id === selectedItem)?.name ?? "";
                            setEnvVar(`${itemName}_${f}`.toUpperCase().replace(/[^A-Z0-9_]/g, "_"));
                          }
                        }}
                        className={`rounded-full px-3.5 py-1.5 text-sm transition ${
                          selectedField === f
                            ? "bg-accent text-surface"
                            : "bg-surface-tertiary text-text-secondary hover:text-text-primary"
                        }`}
                      >
                        {f.toLowerCase()}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {error && <p className="text-xs text-danger">{error}</p>}

      <div className="flex items-center gap-2">
        {!creating && (
          <button
            onClick={() => setCreating(true)}
            className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
          >
            <PlusIcon className="h-4 w-4" />
            New
          </button>
        )}
        <div className="flex-1" />
        <button
          onClick={onClose}
          className="w-24 inline-flex items-center justify-center rounded-lg border border-border py-2 text-sm font-medium text-text-secondary hover:bg-surface-tertiary transition"
        >
          Cancel
        </button>
        <button
          onClick={submit}
          disabled={!selectedItem || (bindingMode === "prefix" && !targetEnvVar && !envVar.trim()) || (bindingMode === "single" && !targetEnvVar && (!envVar.trim() || !selectedField)) || saving}
          className="w-24 inline-flex items-center justify-center rounded-lg bg-accent py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
        >
          {saving ? "Adding..." : "Add"}
        </button>
      </div>
      </>)}
    </div>
    </div>
  );
}

export function CredsSection({ principalKind, principalId }: CredsSectionProps) {
  const [grants, setGrants] = useState<VaultGrant[]>([]);
  const [connections, setConnections] = useState<Map<string, VaultConnection>>(new Map());
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [revoking, setRevoking] = useState(false);
  const [showAdd, setShowAdd] = useState(false);

  const reload = useCallback(() => {
    Promise.all([
      api.get<VaultGrant[]>("/api/vaults/grants"),
      api.get<VaultConnection[]>("/api/vaults"),
    ])
      .then(([allGrants, allConns]) => {
        setGrants(allGrants.filter((g) => matchesPrincipal(g, principalKind, principalId)));
        setConnections(new Map(allConns.map((c) => [c.id, c])));
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [principalKind, principalId]);

  useEffect(() => { reload(); }, [reload]);

  const toggleSelect = useCallback((id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const toggleSelectAll = useCallback(() => {
    setSelected((prev) => {
      if (prev.size === grants.length) return new Set();
      return new Set(grants.map((g) => g.id));
    });
  }, [grants]);

  const revokeSelected = useCallback(async () => {
    if (selected.size === 0) return;
    setRevoking(true);
    try {
      for (const id of selected) {
        await api.delete(`/api/vaults/grants/${id}`);
      }
      setGrants((prev) => prev.filter((g) => !selected.has(g.id)));
      setSelected(new Set());
    } catch {
    } finally {
      setRevoking(false);
    }
  }, [selected]);

  return (
    <div>
      <SectionHeader title="Credentials" description={`Vault access grants for this ${principalKind === "agent" ? "agent" : "server"}`} icon={KeyIcon} />

      {loading && <p className="text-sm text-text-tertiary py-8 text-center">Loading...</p>}

      {!loading && (
        <div className="space-y-3">
          <div className="flex items-center justify-between min-h-[36px]">
            <div className="flex items-center gap-2">
              {grants.length > 0 && (
                <span className="text-sm text-text-tertiary">{grants.length} grant{grants.length !== 1 ? "s" : ""}</span>
              )}
              {selected.size > 0 && (
                <button
                  onClick={revokeSelected}
                  disabled={revoking}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-danger hover:bg-surface-tertiary disabled:opacity-50 transition"
                >
                  <TrashIcon className="h-3.5 w-3.5" />
                  {revoking ? "Revoking..." : `Revoke ${selected.size}`}
                </button>
              )}
            </div>
            {!showAdd && (
              <button
                onClick={() => setShowAdd(true)}
                className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-text-secondary hover:bg-surface-tertiary transition"
              >
                <PlusIcon className="h-3.5 w-3.5" />
                Add credential
              </button>
            )}
          </div>

          {grants.length > 0 && (
            <div className="rounded-xl border border-border bg-surface-secondary divide-y divide-border">
              <div className="px-4 py-2 flex items-center gap-3 bg-surface-tertiary/30">
                <Checkbox.Root
                  checked={grants.every((g) => selected.has(g.id)) ? true : grants.some((g) => selected.has(g.id)) ? "indeterminate" : false}
                  onCheckedChange={toggleSelectAll}
                  className="h-4 w-4 rounded border border-border bg-surface flex items-center justify-center data-[state=checked]:bg-accent data-[state=checked]:border-accent data-[state=indeterminate]:bg-accent data-[state=indeterminate]:border-accent transition shrink-0"
                >
                  <Checkbox.Indicator>
                    {grants.every((g) => selected.has(g.id))
                      ? <CheckIcon className="h-3 w-3 text-surface" />
                      : <MinusIcon className="h-3 w-3 text-surface" />}
                  </Checkbox.Indicator>
                </Checkbox.Root>
                <span className="text-xs text-text-secondary">
                  {selected.size > 0 ? `${selected.size} selected` : "Select all"}
                </span>
              </div>
              {grants.map((grant) => {
                const conn = connections.get(grant.connection_id);
                const provider = conn?.provider ?? "local";
                const ProviderIcon = PROVIDER_ICONS[provider];
                return (
                  <div
                    key={grant.id}
                    onClick={(e) => { if (!(e.target as HTMLElement).closest("button")) toggleSelect(grant.id); }}
                    className="px-4 py-3 flex items-center gap-3 transition hover:bg-surface-tertiary cursor-pointer"
                  >
                    <Checkbox.Root
                      checked={selected.has(grant.id)}
                      onCheckedChange={() => toggleSelect(grant.id)}
                      className="h-4 w-4 rounded border border-border bg-surface flex items-center justify-center data-[state=checked]:bg-accent data-[state=checked]:border-accent transition shrink-0"
                    >
                      <Checkbox.Indicator>
                        <CheckIcon className="h-3 w-3 text-surface" />
                      </Checkbox.Indicator>
                    </Checkbox.Root>
                    <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-surface-tertiary shrink-0">
                      {ProviderIcon
                        ? <ProviderIcon size={18} className="text-text-secondary" />
                        : <KeyIcon className="h-5 w-5 text-text-tertiary" />}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium text-text-primary truncate">{grant.query}</span>
                        <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] font-medium text-text-secondary">
                          {PROVIDER_LABELS[provider] ?? provider}
                        </span>
                      </div>
                      {grant.query && (
                        <div className="text-xs text-text-tertiary font-mono mt-0.5">
                          {grant.query}_*
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          {grants.length === 0 && !showAdd && (
            <p className="text-sm text-text-tertiary py-6 text-center">No credentials assigned.</p>
          )}

          {showAdd && (
            <AddCredentialForm
              connections={connections}
              principalKind={principalKind}
              principalId={principalId}
              existingGrants={grants}
              onClose={() => setShowAdd(false)}
              onCreated={(grant) => {
                setGrants((prev) => [...prev, grant]);
                setShowAdd(false);
              }}
            />
          )}
        </div>
      )}
    </div>
  );
}
