"use client";

import { useState, useEffect } from "react";
import { api } from "@/lib/api-client";
import type { CredentialTarget, GrantDuration, HitlResponse, ToolCall, VaultField } from "@/lib/types";
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
  if (!hitl || hitl.request.type !== "Takeover") return null;
  const { reason, debugger_url } = hitl.request.data;

  return (
    <div className="space-y-2">
      <p className="text-sm text-text-primary">{reason}</p>
      <div className="flex flex-wrap gap-1.5">
        {debugger_url && (
          <a
            href={debugger_url}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary hover:border-accent hover:text-accent transition"
          >
            Open Browser Debugger
          </a>
        )}
        <button
          onClick={() => onResolve({ type: "Choice", data: "Done" }, "Done")}
          className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary hover:border-accent hover:text-accent transition"
        >
          Resume Agent
        </button>
      </div>
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

export function CredentialContent({ te, onResolve }: ToolContentProps) {
  const hitl = te.hitl;
  const queryStr = hitl?.request.type === "Credential" ? hitl.request.data.query : "";
  const reasonStr = hitl?.request.type === "Credential" ? hitl.request.data.reason : "";

  const [connections, setConnections] = useState<VaultConnection[]>([]);
  const [selectedConnection, setSelectedConnection] = useState("");
  const [items, setItems] = useState<VaultItem[]>([]);
  const [selectedItem, setSelectedItem] = useState("");
  const [duration, setDuration] = useState<GrantDuration>("once");
  const [searchQuery, setSearchQuery] = useState(queryStr);
  const [searching, setSearching] = useState(false);
  const [bindingMode, setBindingMode] = useState<"prefix" | "single">("prefix");
  const [envVarPrefix, setEnvVarPrefix] = useState(defaultPrefix(queryStr));
  const [envVar, setEnvVar] = useState("");
  const [fieldKind, setFieldKind] = useState<"Password" | "Username" | "Custom">("Password");
  const [customFieldName, setCustomFieldName] = useState("");

  useEffect(() => {
    api.get<VaultConnection[]>("/api/vaults").then((conns) => {
      const enabled = conns.filter((c) => c.enabled);
      setConnections(enabled);
      if (enabled.length > 0) setSelectedConnection(enabled[0].id);
    });
  }, []);

  useEffect(() => {
    if (!selectedConnection || !searchQuery) return;
    setSearching(true);
    api
      .get<VaultItem[]>(`/api/vaults/${selectedConnection}/items?q=${encodeURIComponent(searchQuery)}`)
      .then((results) => {
        setItems(results);
        if (results.length > 0) setSelectedItem(results[0].id);
      })
      .finally(() => setSearching(false));
  }, [selectedConnection, searchQuery]);

  if (!hitl || hitl.request.type !== "Credential") return null;

  const buildTarget = (): CredentialTarget | null => {
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
  };

  const target = buildTarget();

  const handleApprove = () => {
    if (!selectedItem || !target) return;
    onResolve(
      {
        type: "Vault",
        data: {
          type: "Granted",
          data: {
            connection_id: selectedConnection,
            vault_item_id: selectedItem,
            grant_duration: duration,
            target,
          },
        },
      },
      "Approved",
    );
  };

  const handleDeny = () => {
    onResolve({ type: "Vault", data: { type: "Denied" } }, "Denied");
  };

  const durationValue = typeof duration === "string" ? duration : "hours" in duration ? "hours" : "days";

  return (
    <div className="space-y-3">
      <p className="text-sm text-text-tertiary">{reasonStr}</p>

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
            {items.map((item) => (
              <button
                key={item.id}
                onClick={() => setSelectedItem(item.id)}
                className={`w-full rounded-lg border px-3 py-2 text-left text-sm transition ${
                  selectedItem === item.id
                    ? "border-accent bg-accent/10 text-accent"
                    : "border-border text-text-secondary hover:border-accent"
                }`}
              >
                <span className="font-medium">{item.name}</span>
                {item.username && (
                  <span className="ml-2 text-text-tertiary">({item.username})</span>
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

      <ApprovalButtons loading={false} onApprove={handleApprove} onDeny={handleDeny} approveDisabled={!selectedItem || !target} />
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
      return <CredentialContent {...props} />;
    case "App":
      return <AppContent {...props} />;
    default:
      return null;
  }
}
