"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { getCronRuns } from "@/lib/api-client";
import type { TaskResponse } from "@/lib/types";

const statusStyles: Record<string, string> = {
  pending: "bg-warning-bg text-warning-text",
  inprogress: "bg-info-bg text-info-text",
  completed: "bg-success-bg text-success-text",
  failed: "bg-danger-bg text-danger-text",
  cancelled: "bg-surface-tertiary text-text-secondary",
};

const statusLabels: Record<string, string> = {
  pending: "Pending",
  inprogress: "Running",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

interface CronRunsTableProps {
  cronId: string;
  task: TaskResponse;
}

function formatNextRun(task: TaskResponse): string | null {
  if (task.kind.type !== "Cron" || !task.kind.next_run_at) return null;
  try {
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
      timeZone: task.kind.timezone ?? undefined,
      timeZoneName: "short",
    }).format(new Date(task.kind.next_run_at));
  } catch {
    return task.kind.next_run_at;
  }
}

function CronTemplateHeader({ task }: { task: TaskResponse }) {
  if (task.kind.type !== "Cron") return null;
  const cron = task.kind;
  const nextRun = formatNextRun(task);
  const tz = cron.timezone ?? null;
  const modeLabel = cron.mode === "per_instance" ? "Per instance" : "Singleton";
  const concurrencyLabel = cron.concurrency ?? "replace";

  return (
    <div className="mb-4 rounded-lg border border-border bg-surface-secondary p-4">
      <div className="text-xs font-medium uppercase tracking-wide text-text-tertiary mb-2">
        Instruction
      </div>
      <p className="text-sm text-text-primary whitespace-pre-wrap mb-3">
        {task.description || (
          <span className="text-text-tertiary italic">(no instruction)</span>
        )}
      </p>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-text-tertiary">
        <span>
          <span className="text-text-secondary">Schedule:</span>{" "}
          <code className="rounded bg-surface px-1 py-0.5 font-mono text-[11px]">
            {cron.cron_expression}
          </code>
        </span>
        {tz && (
          <span>
            <span className="text-text-secondary">Timezone:</span> {tz}
          </span>
        )}
        {nextRun && (
          <span>
            <span className="text-text-secondary">Next run:</span> {nextRun}
          </span>
        )}
        <span>
          <span className="text-text-secondary">Mode:</span> {modeLabel}
        </span>
        <span>
          <span className="text-text-secondary">Concurrency:</span> {concurrencyLabel}
        </span>
        {cron.process_result && (
          <span className="rounded bg-info-bg text-info-text px-1.5 py-0.5">
            Delivers results to caller
          </span>
        )}
      </div>
    </div>
  );
}

function formatTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  try {
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    }).format(new Date(iso));
  } catch {
    return iso;
  }
}

function durationMs(run: TaskResponse): string {
  if (run.kind.type !== "CronRun") return "—";
  const start = new Date(run.kind.fire_at).getTime();
  const endIso = run.status === "pending" || run.status === "inprogress" ? null : run.updated_at;
  if (!endIso) return "—";
  const end = new Date(endIso).getTime();
  const ms = Math.max(0, end - start);
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${(ms / 60_000).toFixed(1)}m`;
}

function RunsTable({ runs }: { runs: TaskResponse[] }) {
  const router = useRouter();
  return (
    <div className="rounded-lg border border-border overflow-hidden">
      <table className="w-full text-sm">
        <thead className="bg-surface-secondary text-text-tertiary">
          <tr>
            <th className="text-left px-3 py-2 font-medium">#</th>
            <th className="text-left px-3 py-2 font-medium">Fired</th>
            <th className="text-left px-3 py-2 font-medium">Status</th>
            <th className="text-left px-3 py-2 font-medium">Duration</th>
          </tr>
        </thead>
        <tbody>
          {runs.map((run) => {
            const seq = run.kind.type === "CronRun" ? run.kind.sequence_num : 0;
            const fireAt = run.kind.type === "CronRun" ? run.kind.fire_at : null;
            const statusClass =
              statusStyles[run.status] ?? "bg-surface-tertiary text-text-secondary";
            const statusLabel = statusLabels[run.status] ?? run.status;
            const clickable = !!run.chat_id;
            return (
              <tr
                key={run.id}
                onClick={() => {
                  if (run.chat_id) router.push(`/chat?id=${run.chat_id}`);
                }}
                className={`border-t border-border ${
                  clickable ? "hover:bg-surface-secondary cursor-pointer" : ""
                }`}
              >
                <td className="px-3 py-2 text-text-secondary">#{seq}</td>
                <td className="px-3 py-2 text-text-primary">{formatTime(fireAt)}</td>
                <td className="px-3 py-2">
                  <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${statusClass}`}>
                    {statusLabel}
                  </span>
                </td>
                <td className="px-3 py-2 text-text-tertiary">{durationMs(run)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function CronRunsTable({ cronId, task }: CronRunsTableProps) {
  const [runs, setRuns] = useState<TaskResponse[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const data = await getCronRuns(cronId);
        if (!cancelled) setRuns(data);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : "Failed to load runs");
      }
    }
    load();
    const interval = setInterval(load, 5_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [cronId]);

  const body = (() => {
    if (error) {
      return <p className="text-sm text-danger-text">{error}</p>;
    }
    if (runs === null) {
      return <p className="text-sm text-text-tertiary">Loading runs…</p>;
    }
    if (runs.length === 0) {
      return (
        <p className="text-sm text-text-tertiary">
          No runs yet. Waiting for the next scheduled fire.
        </p>
      );
    }
    return <RunsTable runs={runs} />;
  })();

  return (
    <div className="flex-1 overflow-auto px-4 md:px-6 py-4">
      <div className="mx-auto w-full max-w-3xl">
        <CronTemplateHeader task={task} />
        {body}
      </div>
    </div>
  );
}
