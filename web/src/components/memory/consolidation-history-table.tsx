"use client";

import { Fragment, useEffect, useState } from "react";
import { format } from "date-fns";
import { getPkmConsolidation, getPkmConsolidations, type PkmConsolidationRun, type PkmConsolidationStatus } from "@/lib/api-client";
import { ConsolidationStatusCard } from "./consolidation-status-card";

function label(value: string) {
  return value.replaceAll("_", " ").replace(/^./, (character) => character.toUpperCase());
}

export function ConsolidationHistoryTable() {
  const [runs, setRuns] = useState<PkmConsolidationRun[]>([]);
  const [selected, setSelected] = useState<PkmConsolidationStatus | null>(null);
  const [loadingId, setLoadingId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { runs: result } = await getPkmConsolidations();
        if (!cancelled) setRuns(result);
      } catch { /* The parent settings view owns availability errors. */ }
    };
    void load();
    return () => { cancelled = true; };
  }, []);

  async function select(run: PkmConsolidationRun) {
    if (selected?.id === run.id) { setSelected(null); return; }
    setLoadingId(run.id);
    try { setSelected(await getPkmConsolidation(run.id)); } finally { setLoadingId(null); }
  }

  const latestIsActive = runs[0]?.status === "running" || runs[0]?.status === "retrying";
  const previous = latestIsActive ? runs.slice(1) : runs;
  if (previous.length === 0) return null;

  return (
    <section className="overflow-hidden rounded-xl border border-border bg-surface-secondary">
      <div className="border-b border-border px-4 py-3">
        <h2 className="text-sm font-semibold text-text-primary">Previous consolidation runs</h2>
        <p className="mt-0.5 text-xs text-text-secondary">The most recent retained runs. Select one to inspect its details.</p>
      </div>
      <div>
        <table className="w-full table-fixed text-left text-sm">
          <thead className="bg-surface-tertiary/50 text-xs text-text-tertiary">
            <tr><th className="w-[38%] px-4 py-2 font-medium sm:w-[30%]">Started</th><th className="w-[24%] px-2 py-2 font-medium sm:w-[18%]">Status</th><th className="px-2 py-2 font-medium">Memories</th><th className="hidden px-2 py-2 font-medium md:table-cell">Entities</th><th className="px-2 py-2 font-medium">Pages</th><th className="hidden px-3 py-2 font-medium sm:table-cell">Playbooks</th></tr>
          </thead>
          <tbody className="divide-y divide-border">
            {previous.map((run) => (
              <Fragment key={run.id}>
                <tr onClick={() => void select(run)} className={`cursor-pointer text-text-secondary hover:bg-surface-tertiary/40 ${selected?.id === run.id ? "bg-surface-tertiary/40" : ""}`} aria-selected={selected?.id === run.id} aria-busy={loadingId === run.id}>
                  <td className="truncate px-4 py-3 text-text-primary"><span className="sm:hidden">{format(new Date(run.startedAt ?? run.updatedAt), "PP")}</span><span className="hidden sm:inline">{format(new Date(run.startedAt ?? run.updatedAt), "PPp")}</span></td>
                  <td className="truncate px-2 py-3"><span className={run.status === "failed" ? "text-danger" : ""}>{label(run.status)}</span></td>
                  <td className="px-2 py-3 tabular-nums">{run.memoriesAdded}</td><td className="hidden px-2 py-3 tabular-nums md:table-cell">{run.entitiesChanged}</td><td className="px-2 py-3 tabular-nums">{run.pagesBuilt}</td><td className="hidden px-3 py-3 tabular-nums sm:table-cell">{run.playbooksBuilt}</td>
                </tr>
                {selected?.id === run.id && (
                  <tr><td colSpan={6} className="bg-surface px-4 py-4"><ConsolidationStatusCard value={selected} detailsOnly /></td></tr>
                )}
              </Fragment>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
