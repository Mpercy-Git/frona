"use client";

import { useState, useEffect, useCallback } from "react";
import { format, formatDistanceToNow } from "date-fns";
import { api, API_URL } from "@/lib/api-client";
import { CopyButton } from "@/components/ui/copy-button";

interface TriggerToken {
  id: string;
  name: string;
  prefix: string;
  expires_at: string;
  last_used_at: string | null;
  created_at: string;
}

interface MintedToken extends TriggerToken {
  token: string;
}

type ListedToken = TriggerToken & { expired: boolean };

/** Expiry is worked out when the list loads, not while rendering. */
function withExpiry(tokens: TriggerToken[]): ListedToken[] {
  const now = Date.now();
  return tokens.map((t) => ({ ...t, expired: new Date(t.expires_at).getTime() < now }));
}

const EXPIRY_OPTIONS = [
  { days: 30, label: "30 days" },
  { days: 90, label: "90 days" },
  { days: 365, label: "1 year" },
] as const;

/** Where an outside system sends its trigger. Same origin as the app unless a
 *  separate backend URL is configured. */
function triggerUrl(agentId: string): string {
  const base = API_URL || (typeof window !== "undefined" ? window.location.origin : "");
  return `${base}/api/agents/${agentId}/trigger`;
}

function exampleRequest(agentId: string, token: string): string {
  return [
    `curl -X POST ${triggerUrl(agentId)} \\`,
    `  -H "Authorization: Bearer ${token}" \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -d '{"message": "Doorbell rang"}'`,
  ].join("\n");
}

/**
 * Tokens that let an outside system (a doorbell, a webhook, a sensor) wake this
 * agent. Each one works on this agent's trigger route and nowhere else.
 * Owner-only — the parent hides this section for agents shared with the user.
 */
export function TriggersSection({ agentId }: { agentId: string }) {
  const [tokens, setTokens] = useState<ListedToken[]>([]);
  const [loading, setLoading] = useState(true);
  const [name, setName] = useState("");
  const [days, setDays] = useState<number>(365);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [minted, setMinted] = useState<MintedToken | null>(null);

  const load = useCallback(async () => {
    try {
      setTokens(withExpiry(await api.get<TriggerToken[]>(`/api/agents/${agentId}/trigger-tokens`)));
    } catch {
      setError("Failed to load trigger tokens");
    } finally {
      setLoading(false);
    }
  }, [agentId]);

  useEffect(() => { load(); }, [load]);

  const create = async () => {
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const token = await api.post<MintedToken>(`/api/agents/${agentId}/trigger-tokens`, {
        name: name.trim(),
        expires_in_days: days,
      });
      setMinted(token);
      setName("");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create token");
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (token: TriggerToken) => {
    if (!confirm(`Revoke "${token.name}"? Anything using it will stop working at once.`)) return;
    setError(null);
    try {
      await api.delete(`/api/auth/tokens/${encodeURIComponent(token.id)}`);
      if (minted?.id === token.id) setMinted(null);
      setTokens((current) => current.filter((t) => t.id !== token.id));
    } catch {
      setError("Failed to revoke");
    }
  };

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-base font-semibold text-text-primary">Triggers</h3>
        <p className="text-sm text-text-tertiary mt-1">
          Let an outside system — a doorbell, a webhook, a sensor — wake this agent. Each
          trigger opens a new chat with its message and any images, and the agent starts
          straight away. A token works only for this agent&rsquo;s trigger, so it can&rsquo;t
          open your account.
        </p>
      </div>

      {error && <p className="text-sm text-error-text">{error}</p>}

      <div className="flex flex-wrap items-end gap-2">
        <div className="flex-1 min-w-[12rem]">
          <label htmlFor="trigger-token-name" className="block text-xs font-medium text-text-tertiary mb-1">
            Name
          </label>
          <input
            id="trigger-token-name"
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") create(); }}
            placeholder="e.g. Front doorbell"
            className="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
          />
        </div>
        <div>
          <label htmlFor="trigger-token-expiry" className="block text-xs font-medium text-text-tertiary mb-1">
            Expires after
          </label>
          <select
            id="trigger-token-expiry"
            value={days}
            onChange={(e) => setDays(Number(e.target.value))}
            className="rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent focus:outline-none"
          >
            {EXPIRY_OPTIONS.map((o) => (
              <option key={o.days} value={o.days}>{o.label}</option>
            ))}
          </select>
        </div>
        <button
          onClick={create}
          disabled={!name.trim() || busy}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-surface hover:bg-accent-hover disabled:opacity-50 transition"
        >
          Create token
        </button>
      </div>

      {minted && (
        <div className="rounded-lg border border-accent/40 bg-accent/5 p-3 space-y-3">
          <div className="flex items-start justify-between gap-2">
            <p className="text-sm text-text-primary">
              <span className="font-medium">{minted.name}</span> is ready. Copy it now — it
              won&rsquo;t be shown again.
            </p>
            <button
              onClick={() => setMinted(null)}
              className="text-xs text-text-tertiary hover:text-text-primary"
            >
              Done
            </button>
          </div>
          <div className="flex items-center gap-2">
            <code
              data-testid="minted-token"
              className="flex-1 min-w-0 truncate rounded-md bg-surface px-2 py-1.5 text-xs text-text-primary border border-border"
            >
              {minted.token}
            </code>
            <CopyButton value={minted.token} />
          </div>
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs font-medium text-text-tertiary">Example request</span>
              <CopyButton value={exampleRequest(agentId, minted.token)} size="compact" />
            </div>
            <pre className="overflow-x-auto rounded-md bg-surface px-2 py-1.5 text-xs text-text-secondary border border-border">
              {exampleRequest(agentId, "<token>")}
            </pre>
            <p className="text-xs text-text-tertiary mt-1">
              Add <code>&quot;images&quot;: [&#123;&quot;media_type&quot;: &quot;image/jpeg&quot;, &quot;data&quot;: &quot;&lt;base64&gt;&quot;&#125;]</code> to
              send up to four pictures the agent sees on its first turn.
            </p>
          </div>
        </div>
      )}

      {loading ? (
        <p className="text-sm text-text-tertiary">Loading...</p>
      ) : tokens.length === 0 ? (
        <p className="text-sm text-text-tertiary">No trigger tokens yet.</p>
      ) : (
        <div className="space-y-1">
          {tokens.map((t) => {
            const expired = t.expired;
            return (
              <div
                key={t.id}
                className="flex items-center justify-between gap-3 rounded-lg border border-border bg-surface px-3 py-2"
              >
                <div className="min-w-0">
                  <div className="text-sm text-text-primary truncate">{t.name}</div>
                  <div className="text-xs text-text-tertiary">
                    <code>{t.prefix}</code>
                    {" · "}
                    {t.last_used_at
                      ? `last used ${formatDistanceToNow(new Date(t.last_used_at), { addSuffix: true })}`
                      : "never used"}
                    {" · "}
                    <span className={expired ? "text-error-text" : undefined}>
                      {expired ? "expired" : "expires"} {format(new Date(t.expires_at), "d MMM yyyy")}
                    </span>
                  </div>
                </div>
                <button
                  onClick={() => revoke(t)}
                  className="shrink-0 text-xs text-text-tertiary hover:text-error-text"
                >
                  Revoke
                </button>
              </div>
            );
          })}
        </div>
      )}

      <p className="text-xs text-text-tertiary border-t border-border pt-3">
        Each agent can be triggered at most six times a minute. The agent&rsquo;s reply
        notifies you like any chat reply, showing the trigger&rsquo;s first image.
      </p>
    </div>
  );
}
