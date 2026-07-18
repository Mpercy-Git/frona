"use client";

import { useState, useEffect, useMemo, useCallback } from "react";
import { api } from "@/lib/api-client";
import type { CredentialRequestItem, CredentialTarget, GrantDuration, HitlResponse, ToolCall, VaultField } from "@/lib/types";
import { ApprovalButtons } from "./approval-parts";

function Label({ children }: { children: React.ReactNode }) {
  return <label className="block text-sm font-medium text-text-tertiary mb-1">{children}</label>;
}

export interface ToolContentProps {
  te: ToolCall;
  chatId: string;
  /**
   * Called when the user produces a response. The wizard submits all
   * collected responses in a single batch via the unified resolve endpoint.
   * `displayText` is what we show in the wizard chip for "selected answer".
   */
  onResolve: (response: HitlResponse, displayText: string) => void;
}

export function QuestionContent({ te, onResolve, selectedAnswer }: ToolContentProps & { selectedAnswer?: string }) {
  const hitl = te.hitl;
  if (!hitl || hitl.request.type !== "Question") return null;
  const question = hitl.prompt;
  const options = hitl.request.data.options;

  return (
    <div className="space-y-2">
      <p className="text-sm text-text-primary">{question}</p>
      {options.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {options.map((option) => (
            <button
              key={option}
              onClick={() => onResolve({ type: "Choice", data: option }, option)}
              className={`rounded-lg border px-2.5 py-1 text-xs font-medium transition ${
                selectedAnswer === option
                  ? "border-accent bg-accent/10 text-accent"
                  : "border-border text-text-secondary hover:border-accent hover:text-accent"
              }`}
            >
              {option}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function TakeoverContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  if (!hitl || hitl.request.type !== "Takeover") return null;
  const { reason, debugger_url } = hitl.request.data;

  // The debugger endpoint requires auth, and opening it as a plain link sends
  // no Authorization header (→ "unauthenticated"). Instead, mint a short-lived
  // presigned URL via an authenticated fetch, then open that in a new tab.
  const openDebugger = async () => {
    setOpenError(null);
    setOpening(true);
    try {
      const { url } = await api.get<{ url: string }>(`${debugger_url}/link`);
      window.open(url, "_blank", "noopener,noreferrer");
    } catch (e) {
      setOpenError(e instanceof Error ? e.message : "Failed to open debugger");
    } finally {
      setOpening(false);
    }
  };

  return (
    <div className="space-y-2">
      <p className="text-sm text-text-primary">{reason}</p>
      <div className="flex flex-wrap gap-1.5">
        {debugger_url && (
          <button
            onClick={openDebugger}
            disabled={opening}
            className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary hover:border-accent hover:text-accent disabled:opacity-50 transition"
          >
            {opening ? "Opening…" : "Open Browser Debugger"}
          </button>
        )}
        <button
          onClick={() => onResolve({ type: "Choice", data: "Done" }, "Done")}
          className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary hover:border-accent hover:text-accent transition"
        >
          Resume Agent
        </button>
      </div>
      {openError && <p className="text-xs text-error-text">{openError}</p>}
    </div>
  );
}

interface VaultItem {
  id: string;
  name: string;
  username?: string;
}

interface VaultConnection {
  id: string;
  name: string;
  provider: string;
  enabled: boolean;
}

function defaultPrefix(query: string): string {
  return query.toUpperCase().replace(/[^A-Z0-9]+/g, "_").replace(/^_+|_+$/g, "");
}

interface SlotGrant {
  connection_id: string;
  vault_item_id: string;
  target: CredentialTarget;
}

/**
 * One credential in a (possibly batched) request: pick the vault, find the
 * item, and choose how it's exposed as env vars. Reports the built grant (or
 * `null` while incomplete) to the parent, which collects one per slot and
 * submits them together.
 */
function CredentialSlot({
  item,
  index,
  showHeader,
  connections,
  onChange,
}: {
  item: CredentialRequestItem;
  index: number;
  showHeader: boolean;
  connections: VaultConnection[];
  onChange: (index: number, grant: SlotGrant | null) => void;
}) {
  const [selectedConnection, setSelectedConnection] = useState("");
  const [items, setItems] = useState<VaultItem[]>([]);
  const [selectedItem, setSelectedItem] = useState("");
  const [searchQuery, setSearchQuery] = useState(item.query);
  const [searching, setSearching] = useState(false);
  const [bindingMode, setBindingMode] = useState<"prefix" | "single">("prefix");
  const [envVarPrefix, setEnvVarPrefix] = useState(defaultPrefix(item.query));
  const [envVar, setEnvVar] = useState("");
  const [fieldKind, setFieldKind] = useState<"Password" | "Username" | "Custom">("Password");
  const [customFieldName, setCustomFieldName] = useState("");

  useEffect(() => {
    if (!selectedConnection && connections.length > 0) {
      setSelectedConnection(connections[0].id);
    }
  }, [connections, selectedConnection]);

  useEffect(() => {
    if (!selectedConnection || !searchQuery) return;
    let cancelled = false;
    setSearching(true);
    api
      .get<VaultItem[]>(`/api/vaults/${selectedConnection}/items?q=${encodeURIComponent(searchQuery)}`)
      .then((results) => {
        if (cancelled) return;
        setItems(results);
        setSelectedItem(results.length > 0 ? results[0].id : "");
      })
      .finally(() => {
        if (!cancelled) setSearching(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedConnection, searchQuery]);

  const target = useMemo<CredentialTarget | null>(() => {
    if (bindingMode === "prefix") {
      const prefix = envVarPrefix.trim();
      if (!prefix) return null;
      return { Prefix: { env_var_prefix: prefix } };
    }
    const name = envVar.trim();
    if (!name) return null;
    let field: VaultField;
    if (fieldKind === "Custom") {
      const cn = customFieldName.trim();
      if (!cn) return null;
      field = { Custom: { name: cn } };
    } else {
      field = fieldKind;
    }
    return { Single: { env_var: name, field } };
  }, [bindingMode, envVarPrefix, envVar, fieldKind, customFieldName]);

  useEffect(() => {
    if (!selectedConnection || !selectedItem || !target) {
      onChange(index, null);
    } else {
      onChange(index, { connection_id: selectedConnection, vault_item_id: selectedItem, target });
    }
  }, [selectedConnection, selectedItem, target, index, onChange]);

  return (
    <div className={showHeader ? "space-y-3 rounded-lg border border-border p-3" : "space-y-3"}>
      {showHeader && (
        <p className="text-sm font-medium text-text-primary">
          {index + 1}. {item.label ?? item.query}
        </p>
      )}

      <div>
        <Label>Vault</Label>
        <select
          value={selectedConnection}
          onChange={(e) => setSelectedConnection(e.target.value)}
          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
        >
          {connections.map((c) => (
            <option key={c.id} value={c.id}>{c.name}</option>
          ))}
        </select>
      </div>

      <div>
        <Label>Search</Label>
        <input
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder="Search vault items..."
          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary"
        />
      </div>

      <div>
        <Label>Item</Label>
        {searching ? (
          <p className="text-xs text-text-tertiary py-1">Searching...</p>
        ) : items.length > 0 ? (
          <div className="space-y-1">
            {items.map((vi) => (
              <button
                key={vi.id}
                onClick={() => setSelectedItem(vi.id)}
                className={`w-full rounded-lg border px-3 py-2 text-left text-sm transition ${
                  selectedItem === vi.id
                    ? "border-accent bg-accent/10 text-accent"
                    : "border-border text-text-secondary hover:border-accent"
                }`}
              >
                <span className="font-medium">{vi.name}</span>
                {vi.username && (
                  <span className="ml-2 text-text-tertiary">({vi.username})</span>
                )}
              </button>
            ))}
          </div>
        ) : (
          <p className="text-xs text-text-tertiary py-1">No items found</p>
        )}
      </div>

      <div>
        <Label>Expose as</Label>
        <div className="flex gap-1.5 mb-2">
          <button
            onClick={() => setBindingMode("prefix")}
            className={`flex-1 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition ${
              bindingMode === "prefix"
                ? "border-accent bg-accent/10 text-accent"
                : "border-border text-text-secondary hover:border-accent"
            }`}
          >
            All fields under prefix
          </button>
          <button
            onClick={() => setBindingMode("single")}
            className={`flex-1 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition ${
              bindingMode === "single"
                ? "border-accent bg-accent/10 text-accent"
                : "border-border text-text-secondary hover:border-accent"
            }`}
          >
            One field
          </button>
        </div>
        {bindingMode === "prefix" ? (
          <input
            value={envVarPrefix}
            onChange={(e) => setEnvVarPrefix(e.target.value)}
            placeholder="DB"
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary"
          />
        ) : (
          <div className="space-y-2">
            <input
              value={envVar}
              onChange={(e) => setEnvVar(e.target.value)}
              placeholder="DB_PASSWORD"
              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary"
            />
            <select
              value={fieldKind}
              onChange={(e) => setFieldKind(e.target.value as "Password" | "Username" | "Custom")}
              className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
            >
              <option value="Password">Password</option>
              <option value="Username">Username</option>
              <option value="Custom">Custom field…</option>
            </select>
            {fieldKind === "Custom" && (
              <input
                value={customFieldName}
                onChange={(e) => setCustomFieldName(e.target.value)}
                placeholder="api_key"
                className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm font-mono text-text-primary placeholder:text-text-tertiary"
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * Credential approval. Handles both a single `Credential` request and a
 * batched `Credentials` request — the batch renders one slot per key so the
 * user provides every secret an API needs (app key, user key, …) in one go
 * and a single Duration + Approve resolves them all together.
 */
export function CredentialContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;

  const items = useMemo<CredentialRequestItem[]>(() => {
    if (hitl?.request.type === "Credential") return [{ query: hitl.request.data.query }];
    if (hitl?.request.type === "Credentials") return hitl.request.data.items;
    return [];
  }, [hitl]);

  const reason =
    hitl?.request.type === "Credential"
      ? hitl.request.data.reason
      : hitl?.request.type === "Credentials"
        ? hitl.request.data.reason
        : "";

  const [connections, setConnections] = useState<VaultConnection[]>([]);
  const [duration, setDuration] = useState<GrantDuration>("once");
  const [grants, setGrants] = useState<(SlotGrant | null)[]>(() => items.map(() => null));

  useEffect(() => {
    api.get<VaultConnection[]>("/api/vaults").then((conns) => {
      setConnections(conns.filter((c) => c.enabled));
    });
  }, []);

  // Keep the grants array aligned with the requested items.
  useEffect(() => {
    setGrants((prev) => items.map((_, i) => prev[i] ?? null));
  }, [items]);

  const handleSlotChange = useCallback((index: number, grant: SlotGrant | null) => {
    setGrants((prev) => {
      const next = prev.slice();
      next[index] = grant;
      return next;
    });
  }, []);

  if (!hitl || (hitl.request.type !== "Credential" && hitl.request.type !== "Credentials")) {
    return null;
  }

  const multiple = items.length > 1;
  const allReady = grants.length === items.length && grants.every((g) => g !== null);

  const handleApprove = () => {
    if (!allReady) return;
    const built = items.map((item, i) => {
      const g = grants[i] as SlotGrant;
      return {
        query: item.query,
        connection_id: g.connection_id,
        vault_item_id: g.vault_item_id,
        grant_duration: duration,
        target: g.target,
      };
    });
    onResolve(
      { type: "Vault", data: { type: "GrantedMany", data: { grants: built } } },
      multiple ? `Granted ${built.length}` : "Approved",
    );
  };

  const handleDeny = () => {
    onResolve({ type: "Vault", data: { type: "Denied" } }, "Denied");
  };

  const durationValue = typeof duration === "string" ? duration : "hours" in duration ? "hours" : "days";

  return (
    <div className="space-y-3">
      <p className="text-sm text-text-tertiary">{reason}</p>

      {items.map((item, i) => (
        <CredentialSlot
          key={`${i}-${item.query}`}
          item={item}
          index={i}
          showHeader={multiple}
          connections={connections}
          onChange={handleSlotChange}
        />
      ))}

      <div>
        <Label>Duration</Label>
        <select
          value={durationValue}
          onChange={(e) => {
            const v = e.target.value;
            if (v === "once") setDuration("once");
            else if (v === "permanent") setDuration("permanent");
            else if (v === "hours") setDuration({ hours: 24 });
            else if (v === "days") setDuration({ days: 7 });
          }}
          className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary"
        >
          <option value="once">Allow once</option>
          <option value="hours">Allow for 24 hours</option>
          <option value="days">Allow for 7 days</option>
          <option value="permanent">Allow permanently</option>
        </select>
      </div>

      <ApprovalButtons loading={false} onApprove={handleApprove} onDeny={handleDeny} approveDisabled={!allReady} />
    </div>
  );
}

export function AppContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;
  if (!hitl || hitl.request.type !== "App") return null;
  const { action, manifest } = hitl.request.data;
  const name = String(manifest?.name || manifest?.id || "Unknown service");
  const description = manifest?.description ? String(manifest.description) : null;
  const command = manifest?.command ? String(manifest.command) : null;

  const handleApprove = () => {
    onResolve({ type: "Approval", data: true }, "Approved");
  };

  const handleDeny = () => {
    onResolve({ type: "Approval", data: false }, "Denied");
  };

  return (
    <div className="space-y-3">
      <div>
        <p className="text-sm font-medium text-text-primary capitalize">{action} service: {name}</p>
        {description && <p className="text-xs text-text-tertiary mt-1">{description}</p>}
      </div>
      {command && (
        <div>
          <Label>Command</Label>
          <code className="block rounded-lg border border-border bg-surface-secondary px-3 py-2 text-xs font-mono text-text-secondary overflow-x-auto">
            {command}
          </code>
        </div>
      )}
      <ApprovalButtons loading={false} onApprove={handleApprove} onDeny={handleDeny} />
    </div>
  );
}

export function ToolContentDispatch(props: ToolContentProps & { selectedAnswer?: string }) {
  switch (props.te.hitl?.request.type) {
    case "Question":
      return <QuestionContent {...props} />;
    case "Takeover":
      return <TakeoverContent {...props} />;
    case "Credential":
    case "Credentials":
      return <CredentialContent {...props} />;
    case "App":
      return <AppContent {...props} />;
    default:
      return null;
  }
}
