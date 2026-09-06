"use client";

import { useCallback, useEffect, useState } from "react";
import { BanknotesIcon, ArrowPathIcon } from "@heroicons/react/24/outline";
import { ApiError, getAdminUsage, listCostReports, runCostAnalysis } from "@/lib/api-client";
import type {
  AdminSpendAnalysis,
  CostReport,
  CostRecommendation,
  ProviderSpend,
  RecommendationKind,
} from "@/lib/types";
import { fmtUsd, fmtK, SummaryCard } from "@/app/(main)/usage/widgets";
import { SectionHeader } from "../field";

const WINDOW_DAYS = 30;

const KIND_LABELS: Record<RecommendationKind, string> = {
  switch_model: "Switch model",
  rebalance_provider: "Rebalance provider",
  enable_caching: "Enable caching",
  subscription_underused: "Subscription underused",
  subscription_overrun: "Subscription overrun",
  pricing_gap: "Pricing gap",
};

const BILLING_LABELS: Record<string, string> = {
  metered: "Pay as you go",
  subscription: "Subscription",
  self_hosted: "Self-hosted",
};

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/** Signed monthly delta. Negative saves money, so it reads as a saving. */
function DeltaBadge({ delta }: { delta: number | null | undefined }) {
  if (delta == null) return <span className="text-text-tertiary">—</span>;
  const saving = delta < 0;
  return (
    <span className={saving ? "text-green-600 dark:text-green-400" : "text-text-secondary"}>
      {saving ? "−" : "+"}
      {fmtUsd(Math.abs(delta))}/mo
    </span>
  );
}

function ConfidencePill({ confidence }: { confidence: CostRecommendation["confidence"] }) {
  return (
    <span className="shrink-0 rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] capitalize text-text-tertiary">
      {confidence}
    </span>
  );
}

function ProviderRow({ p }: { p: ProviderSpend }) {
  const kind = p.billing.kind;
  const currency = p.billing.currency ?? "USD";
  return (
    <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 px-4 py-3">
      <span className="text-sm font-medium text-text-primary">{p.provider}</span>
      <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-tertiary">
        {BILLING_LABELS[kind] ?? kind}
      </span>
      <span className="text-sm tabular-nums text-text-secondary">
        {fmtUsd(p.rollup.cost_usd)}
        <span className="text-text-tertiary"> list value</span>
      </span>
      {kind === "subscription" && p.billing.monthly_cost != null && (
        <span className="text-sm text-text-tertiary">
          fee {p.billing.monthly_cost} {currency}/mo
        </span>
      )}
      {p.allowance && (
        <span
          className={`text-sm tabular-nums ${
            p.allowance.exceeded ? "text-amber-600 dark:text-amber-400" : "text-text-tertiary"
          }`}
        >
          {p.allowance.used_pct.toFixed(0)}% of allowance
          {p.allowance.exceeded && p.allowance.overage_is_metered && " — billing overage"}
        </span>
      )}
      <span className="ml-auto text-sm tabular-nums text-text-tertiary">
        {fmtK(p.rollup.calls)} calls
      </span>
    </div>
  );
}

function ReportCard({ report }: { report: CostReport }) {
  return (
    <div className="rounded-xl border border-border bg-surface-secondary overflow-hidden">
      <div className="flex flex-wrap items-baseline justify-between gap-2 border-b border-border px-4 py-3">
        <div>
          <p className="text-sm font-medium text-text-primary">
            {fmtDate(report.window_since)} – {fmtDate(report.window_until)}
          </p>
          <p className="mt-0.5 text-xs text-text-tertiary">
            Filed {fmtDate(report.created_at)} · prices {report.pricing_version}
          </p>
        </div>
        <div className="text-right">
          <p className="text-sm tabular-nums text-text-primary">
            {fmtUsd(report.metered_cost_usd)} metered
          </p>
          {report.estimated_monthly_savings_usd != null && (
            <p className="mt-0.5 text-xs tabular-nums text-text-tertiary">
              <DeltaBadge delta={report.estimated_monthly_savings_usd} /> if applied
            </p>
          )}
        </div>
      </div>

      <p className="whitespace-pre-wrap px-4 py-3 text-sm leading-relaxed text-text-secondary">
        {report.summary}
      </p>

      {report.recommendations.length === 0 ? (
        <p className="border-t border-border px-4 py-3 text-sm text-text-tertiary">
          No changes recommended.
        </p>
      ) : (
        <div className="divide-y divide-border border-t border-border">
          {report.recommendations.map((r, i) => (
            <div key={i} className="px-4 py-3">
              <div className="flex flex-wrap items-baseline gap-2">
                <span className="text-sm font-medium text-text-primary">
                  {KIND_LABELS[r.kind] ?? r.kind}
                </span>
                {r.model_group && (
                  <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[11px] text-text-tertiary">
                    {r.model_group}
                  </span>
                )}
                <ConfidencePill confidence={r.confidence} />
                <span className="ml-auto text-sm tabular-nums">
                  <DeltaBadge delta={r.estimated_monthly_delta_usd} />
                </span>
              </div>
              {r.from && r.to && (
                <p className="mt-1 text-xs tabular-nums text-text-tertiary">
                  {r.from} → {r.to}
                </p>
              )}
              <p className="mt-1 text-sm text-text-secondary">{r.rationale}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function CostReportsSection() {
  const [analysis, setAnalysis] = useState<AdminSpendAnalysis | null>(null);
  const [reports, setReports] = useState<CostReport[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [queued, setQueued] = useState(false);
  const [running, setRunning] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [a, r] = await Promise.all([getAdminUsage(WINDOW_DAYS), listCostReports()]);
      setAnalysis(a);
      setReports(r);
    } catch (e) {
      setError(
        e instanceof ApiError && e.status === 403
          ? "You do not have permission to view server-wide spend."
          : "Failed to load cost data.",
      );
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleRun = async () => {
    setRunning(true);
    setError(null);
    try {
      await runCostAnalysis();
      setQueued(true);
    } catch (e) {
      setError(
        e instanceof ApiError ? e.message : "Failed to queue a cost analysis.",
      );
    } finally {
      setRunning(false);
    }
  };

  const loading = analysis === null && reports === null && error === null;

  return (
    <div className="space-y-6">
      <SectionHeader
        title="Costs"
        description="What this server spends on inference, and what the cost analyst recommends changing."
        icon={BanknotesIcon}
      />

      {error && (
        <p className="rounded-lg border border-border bg-surface-secondary px-4 py-3 text-sm text-text-secondary">
          {error}
        </p>
      )}

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <SummaryCard
          label="Metered spend"
          value={analysis ? fmtUsd(analysis.metered_cost_usd) : "—"}
          hint={`Last ${WINDOW_DAYS} days · money actually spent`}
          loading={loading}
        />
        <SummaryCard
          label="Subscription fees"
          value={analysis ? fmtUsd(analysis.subscription_cost_usd) : "—"}
          hint="Pro-rated to this window"
          loading={loading}
        />
        <SummaryCard
          label="Covered by subscriptions"
          value={analysis ? fmtUsd(analysis.subscription_list_value_usd) : "—"}
          hint="List value of usage the fees already paid for"
          loading={loading}
        />
        <SummaryCard
          label="Self-hosted value"
          value={analysis ? fmtUsd(analysis.self_hosted_list_value_usd) : "—"}
          hint="What the same work would have cost hosted"
          loading={loading}
        />
      </div>

      {analysis && analysis.uncosted_calls > 0 && (
        <p className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-3 text-sm text-text-secondary">
          {fmtK(analysis.uncosted_calls)} call(s) in this window have no catalogue price. Their
          spend is unmeasured rather than zero, so the figures above understate the total.
        </p>
      )}

      {analysis && analysis.providers.length > 0 && (
        <div>
          <h4 className="mb-2 text-sm font-medium text-text-primary">By provider</h4>
          <div className="divide-y divide-border rounded-xl border border-border bg-surface-secondary">
            {analysis.providers.map((p) => (
              <ProviderRow key={p.provider} p={p} />
            ))}
          </div>
        </div>
      )}

      <div>
        <div className="mb-2 flex items-baseline justify-between gap-3">
          <h4 className="text-sm font-medium text-text-primary">Reports</h4>
          <button
            onClick={handleRun}
            disabled={running}
            className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm text-text-secondary transition hover:bg-surface-tertiary hover:text-text-primary disabled:opacity-50"
          >
            <ArrowPathIcon className={`h-4 w-4 ${running ? "animate-spin" : ""}`} />
            Analyse now
          </button>
        </div>

        {queued && (
          <p className="mb-3 rounded-lg border border-border bg-surface-secondary px-4 py-3 text-sm text-text-secondary">
            Analysis queued. The cost analyst runs it in the background and files a report — you
            will get a notification when it is ready. Reload this page to see it.
          </p>
        )}

        {reports === null ? (
          <p className="text-sm text-text-tertiary">{loading ? "Loading…" : ""}</p>
        ) : reports.length === 0 ? (
          <p className="rounded-xl border border-border bg-surface-secondary px-4 py-6 text-sm text-text-tertiary">
            No cost reports yet. The cost analyst files one monthly, or you can run an analysis
            now.
          </p>
        ) : (
          <div className="space-y-4">
            {reports.map((r) => (
              <ReportCard key={r.id} report={r} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
