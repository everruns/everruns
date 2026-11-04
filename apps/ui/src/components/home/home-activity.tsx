"use client";

import Link from "next/link";
import { useMemo } from "react";
import {
  ChartLegend,
  TimeSeriesChart,
  type TimeRow,
  type TimeSeries,
} from "@/components/charts/time-series-chart";
import { number } from "@/components/monitoring/model";
import {
  axisFormatter,
  bucketLabel,
  timelineColors as C,
  timelineRows,
} from "@/components/monitoring/timeline-model";
import type { MonitoringTimeline } from "@/lib/api/types";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";

const SYNC = "home-activity";

/**
 * The compact form of Monitoring → Timeline: the same series, colors and
 * buckets, without selection. Monitoring owns the full view.
 */
export function HomeTimelineCharts({ data }: { data: MonitoringTimeline }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const rows = useMemo(() => timelineRows(data.buckets), [data.buckets]);
  const rebuilt = data.buckets.some((b) => b.backfilled && !b.sampled);
  const common = {
    rows,
    bucketMs: data.bucket_seconds * 1000,
    syncId: SYNC,
    formatX: axisFormatter(data.bucket_seconds, displayLocale),
    formatTooltipTime: bucketLabel(displayLocale, data.bucket_seconds),
  };
  const sessions: TimeSeries[] = [
    { key: "model", label: ui("Model"), color: C.model, kind: "area", stack: "p" },
    { key: "tools", label: ui("Tools"), color: C.tools, kind: "area", stack: "p" },
    { key: "waiting", label: ui("Waiting"), color: C.waiting, kind: "area", stack: "p" },
    { key: "runnable", label: ui("Runnable"), color: C.runnable, kind: "area", stack: "p" },
    ...(rebuilt
      ? [
          {
            key: "rebuilt",
            label: ui("Running (rebuilt)"),
            color: C.rebuilt,
            kind: "area",
            stack: "p",
          } as TimeSeries,
        ]
      : []),
    { key: "peak", label: ui("Peak"), color: C.peak, kind: "line", dashed: true },
  ];
  const turns: TimeSeries[] = [
    { key: "completed", label: ui("Completed"), color: C.completed, kind: "bar", stack: "t" },
    { key: "cancelled", label: ui("Cancelled"), color: C.cancelled, kind: "bar", stack: "t" },
    { key: "failed", label: ui("Failed"), color: C.failed, kind: "bar", stack: "t" },
  ];
  return (
    <ActivityCard monitoring>
      <ChartHead title={ui("Parallel sessions")} series={sessions} />
      <TimeSeriesChart
        {...common}
        series={sessions}
        height={150}
        formatY={(v) => number(v)}
        ariaLabel={ui("Parallel sessions over time")}
      />
      <ChartHead title={ui("Turns")} series={turns} />
      <TimeSeriesChart {...common} series={turns} height={110} ariaLabel={ui("Turns over time")} />
    </ActivityCard>
  );
}

/**
 * Without Monitoring there is no recorded history, but the agents activity
 * buckets still give turns per hour for the last day.
 */
export function HomeHourlyChart({
  hourly,
  windowStart,
}: {
  hourly: Array<{ runs: number; failed: number }>;
  windowStart: string;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const start = Date.parse(windowStart);
  const rows: TimeRow[] = hourly.map((bucket, index) => ({
    t: start + index * 3_600_000,
    completed: bucket.runs - bucket.failed,
    failed: bucket.failed,
  }));
  const series: TimeSeries[] = [
    { key: "completed", label: ui("Turns"), color: C.completed, kind: "bar", stack: "t" },
    { key: "failed", label: ui("Failed"), color: C.failed, kind: "bar", stack: "t" },
  ];
  return (
    <ActivityCard monitoring={false}>
      <ChartHead title={ui("Turns started per hour")} series={series} />
      <TimeSeriesChart
        rows={rows}
        series={series}
        height={150}
        bucketMs={3_600_000}
        // Sub-hour formatting labels clock times, which is what a 24-hour axis wants.
        formatX={axisFormatter(60, displayLocale)}
        formatTooltipTime={bucketLabel(displayLocale, 3600)}
        ariaLabel={ui("Turns over time")}
      />
    </ActivityCard>
  );
}

function ActivityCard({
  monitoring,
  children,
}: {
  monitoring: boolean;
  children: React.ReactNode;
}) {
  const ui = useUiTranslations();
  return (
    <section className="border bg-card" aria-label={ui("Activity")} data-testid="home-activity">
      <header className="flex items-center gap-2.5 border-b px-4 py-3">
        <h2 className="text-sm font-semibold">{ui("Activity")}</h2>
        <span className="text-xs text-muted-foreground">{ui("Last 24 hours")}</span>
        {monitoring && (
          <Link
            href="/monitoring?tab=timeline"
            className="ml-auto text-xs underline underline-offset-4"
          >
            {ui("Open in Monitoring")}
          </Link>
        )}
      </header>
      <div className="space-y-2 px-4 pt-3 pb-4">{children}</div>
    </section>
  );
}

function ChartHead({ title, series }: { title: string; series: TimeSeries[] }) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-2 pt-1">
      <span className="text-[13px] font-semibold">{title}</span>
      <ChartLegend series={series} />
    </div>
  );
}
