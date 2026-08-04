"use client";

import { useState, useEffect, useCallback } from "react";
import { XMarkIcon } from "@heroicons/react/24/outline";
import { api } from "@/lib/api-client";

interface ChatShare {
  recipient_id: string;
  recipient_handle: string;
  recipient_name: string;
  created_at: string;
}

/** Manage who a chat is shared with (read-only). Owner-only — callers must
 *  not render this for a chat shared *with* the current user. */
export function ShareChatModal({
  chatId,
  onClose,
}: {
  chatId: string;
  onClose: () => void;
}) {
  const [shares, setShares] = useState<ChatShare[]>([]);
  const [loading, setLoading] = useState(true);
  const [recipient, setRecipient] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setShares(await api.get<ChatShare[]>(`/api/chats/${chatId}/shares`));
    } catch {
      setError("Failed to load shares");
    } finally {
      setLoading(false);
    }
  }, [chatId]);

  useEffect(() => {
    load();
  }, [load]);

  const addShare = async () => {
    if (!recipient.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setShares(
        await api.post<ChatShare[]>(`/api/chats/${chatId}/shares`, {
          recipient: recipient.trim(),
        }),
      );
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
      setShares(await api.delete<ChatShare[]>(`/api/chats/${chatId}/shares/${encoded}`));
    } catch {
      setError("Failed to revoke");
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4">
      <div className="w-full max-w-md rounded-xl border border-border bg-surface p-5 shadow-xl">
        <div className="flex items-start justify-between">
          <div>
            <h3 className="text-base font-semibold text-text-primary">Share chat</h3>
            <p className="mt-1 text-sm text-text-tertiary">
              Give another user read-only access to this conversation. They can view
              messages but can&rsquo;t send anything or respond to prompts.
            </p>
          </div>
          <button
            onClick={onClose}
            className="rounded p-1 text-text-tertiary hover:text-text-primary transition"
          >
            <XMarkIcon className="h-5 w-5" />
          </button>
        </div>

        {error && <p className="mt-3 text-sm text-error-text">{error}</p>}

        <div className="mt-4 flex items-end gap-2">
          <div className="flex-1">
            <label className="block text-xs font-medium text-text-tertiary mb-1">
              Username or email
            </label>
            <input
              type="text"
              value={recipient}
              onChange={(e) => setRecipient(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") addShare();
              }}
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

        <div className="mt-4">
          {loading ? (
            <p className="text-sm text-text-tertiary">Loading...</p>
          ) : shares.length === 0 ? (
            <p className="text-sm text-text-tertiary">Not shared with anyone yet.</p>
          ) : (
            <div className="space-y-1">
              {shares.map((s) => (
                <div
                  key={s.recipient_id}
                  className="flex items-center justify-between rounded-lg border border-border bg-surface px-3 py-2"
                >
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
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
