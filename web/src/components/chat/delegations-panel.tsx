"use client";

import { useEffect, useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { UsersIcon, ArrowTopRightOnSquareIcon } from "@heroicons/react/24/outline";
import { getDelegations, type DelegationInfo } from "@/lib/api-client";

const STATUS_LABEL: Record<DelegationInfo["status"], string> = {
  pending: "Queued",
  inprogress: "Running",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

const STATUS_BADGE: Record<DelegationInfo["status"], string> = {
  pending: "bg-surface-tertiary text-text-secondary",
  inprogress: "bg-blue-400/15 text-blue-400",
  completed: "bg-green-500/15 text-green-500",
  failed: "bg-red-500/15 text-red-500",
  cancelled: "bg-surface-tertiary text-text-tertiary",
};

const ACTIVE: DelegationInfo["status"][] = ["pending", "inprogress"];

/**
 * Delegation observability: shows the sub-tasks the current chat delegated to
 * other agents, with live status and a link into each delegate's own chat, so
 * a parent conversation isn't a black box while its delegates run (or wait on
 * a human).
 */
export function DelegationsPanel({ chatId }: { chatId: string }) {
  const [delegations, setDelegations] = useState<DelegationInfo[]>([]);
  const router = useRouter();

  const refresh = useCallback(async () => {
    try {
      setDelegations(await getDelegations(chatId));
    } catch {
      // best-effort; leave prior state
    }
  }, [chatId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Poll while any delegate is still active so status stays live.
  const hasActive = delegations.some((d) => ACTIVE.includes(d.status));
  useEffect(() => {
    if (!hasActive) return;
    const t = setInterval(refresh, 8000);
    return () => clearInterval(t);
  }, [hasActive, refresh]);

  if (delegations.length === 0) return null;

  return (
    <div className="mx-auto w-full max-w-3xl px-4 pt-2">
      <div className="rounded-lg border border-border bg-surface-secondary">
        <div className="flex items-center gap-2 px-3 py-2 border-b border-border">
          <UsersIcon className="h-4 w-4 text-text-tertiary" />
          <span className="text-xs font-medium text-text-secondary">
            Delegated tasks ({delegations.length})
          </span>
        </div>
        <div className="divide-y divide-border">
          {delegations.map((d) => (
            <div key={d.task_id} className="flex items-center gap-2 px-3 py-2">
              <span className="flex-1 min-w-0 truncate text-sm text-text-primary">
                {d.agent_name ?? d.agent_id}
              </span>
              <span
                className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${STATUS_BADGE[d.status]}`}
              >
                {STATUS_LABEL[d.status]}
              </span>
              {d.chat_id && (
                <button
                  onClick={() => router.push(`/chat?id=${d.chat_id}`)}
                  title="Open delegate's chat"
                  className="rounded p-1 text-text-tertiary hover:text-accent hover:bg-surface-tertiary transition"
                >
                  <ArrowTopRightOnSquareIcon className="h-4 w-4" />
                </button>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
