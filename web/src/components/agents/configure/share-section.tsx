"use client";

import { useState, useEffect, useCallback } from "react";
import { api } from "@/lib/api-client";

interface AgentShare {
  recipient_id: string;
  recipient_handle: string;
  recipient_name: string;
  level: string;
  delegate_credentials: boolean;
  created_at: string;
}

/**
 * Manage who an agent is shared with (use-only). Owner-only — the parent hides
 * this section for agents shared *with* the current user.
 */
export function ShareSection({ agentId }: { agentId: string }) {
  const [shares, setShares] = useState<AgentShare[]>([]);
  const [loading, setLoading] = useState(true);
  const [recipient, setRecipient] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setShares(await api.get<AgentShare[]>(`/api/agents/${agentId}/shares`));
    } catch {
      setError("Failed to load shares");
    } finally {
      setLoading(false);
    }
  }, [agentId]);

  useEffect(() => { load(); }, [load]);

  const addShare = async () => {
    if (!recipient.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setShares(await api.post<AgentShare[]>(`/api/agents/${agentId}/shares`, {
        recipient: recipient.trim(),
      }));
      setRecipient("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to share");
    } finally {
      setBusy(false);
    }
  };

  const removeShare = async (recipientId: string) => {
    setError(null);
    try {
      const encoded = encodeURIComponent(recipientId);
      setShares(await api.delete<AgentShare[]>(`/api/agents/${agentId}/shares/${encoded}`));
    } catch {
      setError("Failed to revoke");
    }
  };

  const toggleDelegation = async (recipientId: string, delegate: boolean) => {
    setError(null);
    try {
      const encoded = encodeURIComponent(recipientId);
      setShares(await api.put<AgentShare[]>(`/api/agents/${agentId}/shares/${encoded}`, {
        delegate_credentials: delegate,
      }));
    } catch {
      setError("Failed to update");
    }
  };

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-base font-semibold text-text-primary">Share</h3>
        <p className="text-sm text-text-tertiary mt-1">
          Give another user access to use this agent. They can chat with it but can&rsquo;t
          edit it, and their runs use their own sandbox and credentials.
        </p>
      </div>

      {error && <p className="text-sm text-error-text">{error}</p>}

      <div className="flex items-end gap-2">
        <div className="flex-1">
          <label className="block text-xs font-medium text-text-tertiary mb-1">
            Username or email
          </label>
          <input
            type="text"
            value={recipient}
            onChange={(e) => setRecipient(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") addShare(); }}
            placeholder="e.g. jane or jane@example.com"
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
          />
        </div>
        <button
          onClick={addShare}
          disabled={!recipient.trim() || busy}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
        >
          Share
        </button>
      </div>

      {loading ? (
        <p className="text-sm text-text-tertiary">Loading...</p>
      ) : shares.length === 0 ? (
        <p className="text-sm text-text-tertiary">Not shared with anyone yet.</p>
      ) : (
        <div className="space-y-1">
          {shares.map((s) => (
            <div
              key={s.recipient_id}
              className="rounded-lg border border-border bg-surface px-3 py-2"
            >
              <div className="flex items-center justify-between">
                <div className="min-w-0">
                  <span className="text-sm text-text-primary">@{s.recipient_handle}</span>
                  {s.recipient_name && (
                    <span className="text-sm text-text-secondary"> — {s.recipient_name}</span>
                  )}
                </div>
                <button
                  onClick={() => removeShare(s.recipient_id)}
                  className="text-xs text-text-tertiary hover:text-error-text"
                >
                  Revoke
                </button>
              </div>
              <label className="mt-2 flex items-center gap-2 text-xs text-text-secondary">
                <input
                  type="checkbox"
                  checked={s.delegate_credentials}
                  onChange={(e) => toggleDelegation(s.recipient_id, e.target.checked)}
                  className="h-3.5 w-3.5 rounded border-border text-accent focus:ring-accent"
                />
                Let their runs use this agent&rsquo;s credentials (yours). They&rsquo;ll be able
                to use — and potentially view — those secrets.
              </label>
            </div>
          ))}
        </div>
      )}

      <p className="text-xs text-text-tertiary border-t border-border pt-3">
        Files: a shared agent runs under this agent&rsquo;s Sandbox settings, so any read/write
        paths you grant it there are available to recipients too.
      </p>
    </div>
  );
}
