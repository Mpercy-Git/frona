"use client";

import { useMemo } from "react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip as RTooltip,
  XAxis,
  YAxis,
} from "recharts";

import { useUsage } from "../usage-context";
import {
  CHART_TOOLTIP_STYLE,
  Card,
  EmptyState,
  SectionHeader,
  SummaryCard,
  fmtBucketLabel,
  fmtMs,
} from "../widgets";

export default function LatencyPage() {
  const { range, data, loading } = useUsage();
  const lat = data?.latency;

  const durSeries = useMemo(() => {
    if (!data?.latency_series) return [];
    return data.latency_series.map((b) => ({
      label: fmtBucketLabel(b.bucket, range.bucket),
      p50: b.duration_ms_p50,
      p95: b.duration_ms_p95,
      p99: b.duration_ms_p99,
    }));
  }, [data, range]);

  const ttftSeries = useMemo(() => {
    if (!data?.latency_series) return [];
    return data.latency_series
      .map((b) => ({
        label: fmtBucketLabel(b.bucket, range.bucket),
        p50: b.ttft_ms_p50,
        p95: b.ttft_ms_p95,
        p99: b.ttft_ms_p99,
      }))
      .filter((b) => b.p50 != null || b.p95 != null || b.p99 != null);
  }, [data, range]);

  const byModel = useMemo(() => {
    if (!data?.latency_by_model) return [];
    return [...data.latency_by_model].sort(
      (a, b) => (b.duration_ms_p95 ?? 0) - (a.duration_ms_p95 ?? 0),
    );
  }, [data]);

  return (
    <div className="mx-auto max-w-6xl px-4 py-6 md:px-8 md:py-10">
      <div className="mb-6">
        <h1 className="text-2xl font-semibold text-text-primary">Latency</h1>
        <p className="text-sm text-text-tertiary">
          Wall-time percentiles across all API requests in the window.
        </p>
      </div>

      <div className="mb-6">
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-tertiary">
          Total call duration
        </h3>
        <p className="mb-3 text-xs text-text-tertiary">
          Time from request send to final byte, including retries and backoff sleeps.
        </p>
        <div className="grid grid-cols-3 gap-3">
          <SummaryCard label="p50" value={fmtMs(lat?.duration_ms_p50)} loading={loading && !data} />
          <SummaryCard label="p95" value={fmtMs(lat?.duration_ms_p95)} loading={loading && !data} />
          <SummaryCard label="p99" value={fmtMs(lat?.duration_ms_p99)} loading={loading && !data} />
        </div>
      </div>

      <Card className="mb-6">
        <SectionHeader title="Duration over time" subtitle="p50 / p95 / p99 per bucket." />
        <div className="mt-4 h-60">
          {durSeries.length === 0 ? (
            <EmptyState loading={loading} />
          ) : (
            <PercentileTrendChart data={durSeries} />
          )}
        </div>
      </Card>

      <div className="mb-6">
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-tertiary">
          Time to first token (TTFT)
        </h3>
        <p className="mb-3 text-xs text-text-tertiary">
          From request send to the first streamed chunk — the &ldquo;feel responsive&rdquo; metric.
          Non-streaming requests (title gen, router) don&apos;t contribute.
        </p>
        <div className="grid grid-cols-3 gap-3">
          <SummaryCard label="p50" value={fmtMs(lat?.ttft_ms_p50)} loading={loading && !data} />
          <SummaryCard label="p95" value={fmtMs(lat?.ttft_ms_p95)} loading={loading && !data} />
          <SummaryCard label="p99" value={fmtMs(lat?.ttft_ms_p99)} loading={loading && !data} />
        </div>
      </div>

      <Card className="mb-6">
        <SectionHeader title="TTFT over time" subtitle="p50 / p95 / p99 per bucket — streaming requests only." />
        <div className="mt-4 h-60">
          {ttftSeries.length === 0 ? (
            <EmptyState loading={loading} />
          ) : (
            <PercentileTrendChart data={ttftSeries} />
          )}
        </div>
      </Card>

      <Card>
        <SectionHeader title="By model" subtitle="Duration p50/p95/p99 and TTFT per model_ref." />
        {byModel.length === 0 ? (
          <div className="mt-4 h-32">
            <EmptyState loading={loading} />
          </div>
        ) : (
          <div className="mt-4 overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-border text-xs uppercase tracking-wide text-text-tertiary">
                  <th className="pb-2 text-left font-medium">Model</th>
                  <th className="pb-2 text-right font-medium">dur p50</th>
                  <th className="pb-2 text-right font-medium">dur p95</th>
                  <th className="pb-2 text-right font-medium">dur p99</th>
                  <th className="pb-2 text-right font-medium">ttft p50</th>
                  <th className="pb-2 text-right font-medium">ttft p95</th>
                  <th className="pb-2 text-right font-medium">ttft p99</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {byModel.map((m) => (
                  <tr key={m.model_ref}>
                    <td className="py-2 pr-3 text-text-primary truncate">{m.model_ref}</td>
                    <td className="py-2 text-right tabular-nums text-text-secondary">{fmtMs(m.duration_ms_p50)}</td>
                    <td className="py-2 text-right tabular-nums text-text-secondary">{fmtMs(m.duration_ms_p95)}</td>
                    <td className="py-2 text-right tabular-nums text-text-secondary">{fmtMs(m.duration_ms_p99)}</td>
                    <td className="py-2 text-right tabular-nums text-text-secondary">{fmtMs(m.ttft_ms_p50)}</td>
                    <td className="py-2 text-right tabular-nums text-text-secondary">{fmtMs(m.ttft_ms_p95)}</td>
                    <td className="py-2 text-right tabular-nums text-text-secondary">{fmtMs(m.ttft_ms_p99)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </div>
  );
}

function PercentileTrendChart({
  data,
}: {
  data: { label: string; p50: number | null; p95: number | null; p99: number | null }[];
}) {
  return (
    <ResponsiveContainer width="100%" height="100%">
      <LineChart data={data} margin={{ top: 8, right: 12, left: 0, bottom: 0 }}>
        <CartesianGrid stroke="var(--border)" strokeOpacity={0.5} vertical={false} />
        <XAxis
          dataKey="label"
          stroke="var(--text-tertiary)"
          fontSize={11}
          tickLine={false}
          axisLine={false}
        />
        <YAxis
          stroke="var(--text-tertiary)"
          fontSize={11}
          tickLine={false}
          axisLine={false}
          tickFormatter={(v) => fmtMs(v)}
        />
        <RTooltip
          contentStyle={CHART_TOOLTIP_STYLE}
          formatter={((v: number) => fmtMs(v)) as never}
        />
        <Line type="monotone" dataKey="p50" stroke="var(--text-tertiary)" strokeWidth={1.5} dot={false} isAnimationActive={false} connectNulls />
        <Line type="monotone" dataKey="p95" stroke="var(--text-secondary)" strokeWidth={1.5} dot={false} isAnimationActive={false} connectNulls />
        <Line type="monotone" dataKey="p99" stroke="var(--accent)" strokeWidth={2} dot={false} isAnimationActive={false} connectNulls />
      </LineChart>
    </ResponsiveContainer>
  );
}
