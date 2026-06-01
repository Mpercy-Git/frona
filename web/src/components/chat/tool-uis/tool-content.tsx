"use client";

import { useState, useEffect } from "react";
import { api } from "@/lib/api-client";
import type { ToolCall } from "@/lib/types";
import { ApprovalButtons } from "./approval-parts";

function Label({ children }: { children: React.ReactNode }) {
  return <label className="block text-sm font-medium text-text-tertiary mb-1">{children}</label>;
}

export interface ToolContentProps {
  te: ToolCall;
  chatId: string;
  onSuccess: (response: string, callback?: () => Promise<void>) => void;
  onFailure: (response: string, callback?: () => Promise<void>) => void;
}

export function QuestionContent({ te, onSuccess, selectedAnswer }: ToolContentProps & { selectedAnswer?: string }) {
  const data = te.tool_data?.data as Record<string, unknown> | undefined;
  if (!data) return null;
  const question = data.question as string;
  const options = (data.options as string[]) ?? [];

  return (
    <div className="space-y-2">
      <p className="text-sm text-text-primary">{question}</p>
      {options.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {options.map((option) => (
            <button
              key={option}
              onClick={() => onSuccess(option)}
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

export function HumanInTheLoopContent({ te, onSuccess }: ToolContentProps) {
  const data = te.tool_data?.data as Record<string, unknown> | undefined;
  if (!data) return null;
  const reason = data.reason as string;
  const debuggerUrl = data.debugger_url as string | undefined;

  return (
    <div className="space-y-2">
      <p className="text-sm text-text-primary">{reason}</p>
      <div className="flex flex-wrap gap-1.5">
        {debuggerUrl && (
          <a
            href={debuggerUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium text-text-secondary hover:border-accent hover:text-accent transition"
          >
            Open Browser Debugger
          </a>
        )}
        <button
          onClick={() => onSuccess("resumed")}
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

type GrantDuration = "once" | { hours: number } | { days: number } | "permanent";

export function VaultApprovalContent({ te, chatId, onSuccess, onFailure }: ToolContentProps) {
  const data = te.tool_data?.data as Record<string, unknown> | undefined;
  const query = (data?.query as string) ?? "";
  const reason = data?.reason as string;
  const envVarPrefix = data?.env_var_prefix as string | null;

  const [connections, setConnections] = useState<VaultConnection[]>([]);
  const [selectedConnection, setSelectedConnection] = useState("");
  const [items, setItems] = useState<VaultItem[]>([]);
  const [selectedItem, setSelectedItem] = useState("");
  const [duration, setDuration] = useState<GrantDuration>("once");
  const [searchQuery, setSearchQuery] = useState(query);
  const [searching, setSearching] = useState(false);

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

  if (!data) return null;

  const handleApprove = () => {
    if (!selectedItem) return;
    onSuccess("approved", () =>
      api.post("/api/vaults/approve", {
        chat_id: chatId,
        connection_id: selectedConnection,
        vault_item_id: selectedItem,
        grant_duration: duration,
        env_var_prefix: envVarPrefix,
      }),
    );
  };

  const handleDeny = () => {
    onFailure("denied", () => api.post("/api/vaults/deny", { chat_id: chatId }));
  };

  const durationValue = typeof duration === "string" ? duration : "hours" in duration ? "hours" : "days";

  return (
    <div className="space-y-3">
      <p className="text-sm text-text-tertiary">{reason}</p>

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

      <ApprovalButtons loading={false} onApprove={handleApprove} onDeny={handleDeny} approveDisabled={!selectedItem} />
    </div>
  );
}

export function ServiceApprovalContent({ te, chatId, onSuccess, onFailure }: ToolContentProps) {
  const data = te.tool_data?.data as Record<string, unknown> | undefined;
  if (!data) return null;
  const action = data.action as string;
  const manifest = data.manifest as Record<string, unknown> | undefined;
  const name = String(manifest?.name || manifest?.id || "Unknown service");
  const description = manifest?.description ? String(manifest.description) : null;
  const command = manifest?.command ? String(manifest.command) : null;

  const handleApprove = () => {
    onSuccess("approved", () => api.post("/api/apps/approve", { chat_id: chatId }));
  };

  const handleDeny = () => {
    onFailure("denied", () => api.post("/api/apps/deny", { chat_id: chatId }));
  };

  return (
    <div className="space-y-3">
      <div>
        <p className="text-sm font-medium text-text-primary">{name}</p>
        {description && <p className="text-xs text-text-tertiary mt-0.5">{description}</p>}
      </div>

      {command && (
        <div>
          <Label>Command</Label>
          <code className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary block">{command}</code>
        </div>
      )}

      <ApprovalButtons loading={false} onApprove={handleApprove} onDeny={handleDeny} />
    </div>
  );
}

export function ToolContentDispatch(props: ToolContentProps & { selectedAnswer?: string }) {
  switch (props.te.tool_data?.type) {
    case "Question":
      return <QuestionContent {...props} />;
    case "HumanInTheLoop":
      return <HumanInTheLoopContent {...props} />;
    case "VaultApproval":
      return <VaultApprovalContent {...props} />;
    case "ServiceApproval":
      return <ServiceApprovalContent {...props} />;
    default:
      return null;
  }
}
