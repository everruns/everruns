"use client";

// Monitoring → Timeline: recorded history over a chosen range
// (knowledge/operations/monitoring.md). Execution shows now; Timeline shows
// the same population, turns, queue, and worker load over time.

import Link from "next/link";
import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
} from "@/components/layout/page-layout";
import {
  ChartLegend,
  TimeSeriesChart,
  type TimeSelection,
  type TimeSeries,
} from "@/components/charts/time-series-chart";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import {
  getMonitoringTimeline,
  getMonitoringTimelineWindow,
  type MonitoringScope,
  type TimelineBounds,
  type TimelineRange,
} from "@/lib/api/monitoring";
import type { MonitoringTimeline, TimelineSummary } from "@/lib/api/types";
import { AlertTriangle } from "lucide-react";
import { duration, number } from "./model";
import {
  axisFormatter,
  bucketLabel,
  change,
  selectionTotals,
  timelineColors as C,
  timelineRanges,
  timelineRows,
  usd,
} from "./timeline-model";

const SYNC = "monitoring-timeline";

export function MonitoringTimelineView({ scope }: { scope: MonitoringScope }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const [bounds, setBounds] = useState<TimelineBounds>({ range: "24h" });
  const [selection, setSelection] = useState<TimeSelection | null>(null);
  const zoomed = !!bounds.from;
  const timeline = useQuery({
    queryKey: ["monitoring-timeline", scope, bounds],
    queryFn: () => getMonitoringTimeline(scope, bounds),
    refetchInterval: zoomed
      ? false
      : bounds.range === "15m" || bounds.range === "1h"
        ? 10_000
        : 60_000,
    retry: false,
  });
  const data = timeline.data;
  const choose = (range: TimelineRange) => {
    setBounds({ range });
    setSelection(null);
  };
  return (
    <div className="space-y-4" data-testid="monitoring-timeline">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex" role="group" aria-label={ui("Time range")}>
          {timelineRanges.map((r) => (
            <Button
              key={r}
              variant="outline"
              size="sm"
              aria-pressed={!zoomed && bounds.range === r}
              onClick={() => choose(r)}
              className={`-ml-px first:ml-0 ${!zoomed && bounds.range === r ? "relative z-10 border-primary font-semibold" : "text-muted-foreground"}`}
            >
              {r}
            </Button>
          ))}
          {zoomed && (
            <Button variant="ghost" size="sm" className="ml-2" onClick={() => choose(bounds.range)}>
              {ui("Reset zoom")}
            </Button>
          )}
        </div>
        {data && (
          <span className="text-xs text-muted-foreground">
            {rangeLabel(data, displayLocale)} ·{" "}
            {ui("{value0}-minute buckets", { value0: String(data.bucket_seconds / 60) })}
          </span>
        )}
      </div>
      {!data ? (
        timeline.error ? (
          <EmptyState
            icon={<AlertTriangle />}
            title={ui("History could not load")}
            description={ui("Refresh when the service is available.")}
            action={<Button onClick={() => timeline.refetch()}>{ui("Retry")}</Button>}
          />
        ) : (
          <Skeleton className="h-[600px]" />
        )
      ) : (
        <TimelineBody
          scope={scope}
          data={data}
          selection={selection}
          onSelect={setSelection}
          onZoom={(from, to) => {
            setBounds({ range: bounds.range, from, to });
            setSelection(null);
          }}
        />
      )}
    </div>
  );
}

function rangeLabel(data: MonitoringTimeline, locale: string) {
  const f = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  return `${f.format(new Date(data.from))} – ${f.format(new Date(data.to))}`;
}

function TimelineBody({
  scope,
  data,
  selection,
  onSelect,
  onZoom,
}: {
  scope: MonitoringScope;
  data: MonitoringTimeline;
  selection: TimeSelection | null;
  onSelect: (s: TimeSelection | null) => void;
  onZoom: (from: string, to: string) => void;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const fleet = scope === "fleet";
  const rows = useMemo(() => timelineRows(data.buckets), [data.buckets]);
  const bucketMs = data.bucket_seconds * 1000;
  const formatX = axisFormatter(data.bucket_seconds, displayLocale);
  const tooltipTime = bucketLabel(displayLocale, data.bucket_seconds);
  const rebuilt = data.buckets.some((b) => b.backfilled && !b.sampled);
  const markers = data.markers
    .filter((m) => fleet || m.kind === "deploy")
    .map((m) => ({
      at: Date.parse(m.at),
      label:
        m.kind === "deploy"
          ? ui("deploy {value0}", { value0: m.label })
          : m.kind === "worker_joined"
            ? ui("{value0} joined", { value0: m.label })
            : ui("{value0} left", { value0: m.label }),
    }));
  const common = {
    rows,
    bucketMs,
    syncId: SYNC,
    formatX,
    formatTooltipTime: tooltipTime,
    selection,
    onSelect,
  };
  const sessions: TimeSeries[] = [
    { key: "model", label: ui("Model"), color: C.model, kind: "area", stack: "p" },
    { key: "tools", label: ui("Tools"), color: C.tools, kind: "area", stack: "p" },
    { key: "waiting", label: ui("Waiting"), color: C.waiting, kind: "area", stack: "p" },
    { key: "runnable", label: ui("Runnable"), color: C.runnable, kind: "area", stack: "p" },
    { key: "setup", label: ui("Setup / handoff"), color: C.setup, kind: "area", stack: "p" },
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
    ...(fleet
      ? [
          {
            key: "capacity",
            label: ui("Worker capacity"),
            color: C.capacity,
            kind: "line",
          } as TimeSeries,
        ]
      : []),
  ];
  const turns: TimeSeries[] = [
    { key: "completed", label: ui("Completed"), color: C.completed, kind: "bar", stack: "t" },
    { key: "cancelled", label: ui("Cancelled"), color: C.cancelled, kind: "bar", stack: "t" },
    { key: "failed", label: ui("Failed"), color: C.failed, kind: "bar", stack: "t" },
  ];
  const bucketName =
    data.bucket_seconds >= 3600
      ? ui("per hour")
      : ui("per {value0} minutes", { value0: String(data.bucket_seconds / 60) });
  return (
    <PageColumns>
      <PageMain>
        <Kpis summary={data.summary} previous={data.previous} fleet={fleet} />
        <ChartCard
          title={fleet ? ui("Parallel sessions against capacity") : ui("Parallel sessions")}
          description={
            fleet
              ? ui(
                  "All organizations, average per bucket by phase; dashed line is the peak, grey line is worker slots.",
                )
              : ui(
                  "Sessions with an active turn, average per bucket by phase; dashed line is the peak.",
                )
          }
          series={sessions}
        >
          <TimeSeriesChart
            {...common}
            series={sessions}
            height={220}
            markers={markers}
            formatY={(v) => number(v)}
            ariaLabel={ui("Parallel sessions over time")}
          />
        </ChartCard>
        {fleet && <WorkerStrip data={data} formatX={formatX} />}
        <ChartCard
          title={ui("Turns")}
          description={ui("Turns finished by outcome, {value0}.", { value0: bucketName })}
          series={turns}
        >
          <TimeSeriesChart
            {...common}
            series={turns}
            height={140}
            ariaLabel={ui("Turns over time")}
          />
        </ChartCard>
        <ChartCard
          title={ui("Waiting for a worker")}
          description={ui("Longest time a ready turn waited for a worker lease in each bucket.")}
          series={[{ key: "wait", label: ui("Longest wait"), color: C.wait }]}
        >
          <TimeSeriesChart
            {...common}
            series={[{ key: "wait", label: ui("Longest wait"), color: C.wait, kind: "line" }]}
            height={100}
            formatY={(v) => `${number(v)}s`}
            ariaLabel={ui("Wait for a worker over time")}
          />
        </ChartCard>
        <div className="grid gap-4 md:grid-cols-2">
          <SmallChart
            title={ui("Tool calls")}
            description={ui("Started, {value0}; failures in red.", { value0: bucketName })}
            common={common}
            series={[
              { key: "tool_calls", label: ui("Tool calls"), color: C.tools, kind: "line" },
              { key: "tool_failures", label: ui("Failed"), color: C.failed, kind: "line" },
            ]}
          />
          <SmallChart
            title={ui("Model spend")}
            description={ui("USD, {value0}.", { value0: bucketName })}
            common={common}
            formatY={(v) => usd(v)}
            series={[{ key: "spend", label: ui("Spend"), color: C.spend, kind: "line" }]}
            footer={
              <Link href="/reports" className="text-xs underline-offset-4 hover:underline">
                {ui("Open Reports →")}
              </Link>
            }
          />
          <SmallChart
            title={ui("Errors")}
            description={ui("Failed turns and failed tool calls.")}
            common={common}
            series={[{ key: "errors", label: ui("Errors"), color: C.failed, kind: "line" }]}
          />
          <SmallChart
            title={ui("Model latency")}
            description={ui("Model call duration, p95.")}
            common={common}
            formatY={(v) => `${number(v)}s`}
            series={[{ key: "latency", label: ui("p95"), color: C.latency, kind: "line" }]}
          />
          {fleet && (
            <SmallChart
              title={ui("Lease takeovers")}
              description={ui("Expired leases another worker picked up.")}
              common={common}
              series={[{ key: "takeovers", label: ui("Takeovers"), color: C.wait, kind: "bar" }]}
            />
          )}
        </div>
        {rebuilt && (
          <p className="text-xs text-muted-foreground">
            {ui(
              "Grey history was rebuilt from turn records before recording began: it shows running turns and outcomes, without the phase split, queue, or worker load.",
            )}
          </p>
        )}
      </PageMain>
      <PageRail>
        <SelectionRail
          scope={scope}
          data={data}
          selection={selection}
          onClear={() => onSelect(null)}
          onZoom={onZoom}
          tooltipTime={tooltipTime}
        />
      </PageRail>
    </PageColumns>
  );
}

function ChartCard({
  title,
  description,
  series,
  children,
}: {
  title: string;
  description: string;
  series: { key: string; label: string; color: string }[];
  children: React.ReactNode;
}) {
  return (
    <section className="border bg-card p-4">
      <div className="mb-2 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">{title}</h2>
          <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
        </div>
        <ChartLegend series={series} />
      </div>
      {children}
    </section>
  );
}

function SmallChart({
  title,
  description,
  series,
  common,
  formatY,
  footer,
}: {
  title: string;
  description: string;
  series: TimeSeries[];
  common: Omit<React.ComponentProps<typeof TimeSeriesChart>, "series" | "ariaLabel">;
  formatY?: (v: number) => string;
  footer?: React.ReactNode;
}) {
  return (
    <section className="border bg-card p-4">
      <h2 className="text-sm font-semibold">{title}</h2>
      <p className="mt-0.5 mb-2 text-xs text-muted-foreground">{description}</p>
      <TimeSeriesChart
        {...common}
        series={series}
        height={100}
        formatY={formatY}
        ariaLabel={title}
      />
      {footer && <div className="mt-2">{footer}</div>}
    </section>
  );
}

function Delta({ value, worseWhenUp = false }: { value: number | null; worseWhenUp?: boolean }) {
  const ui = useUiTranslations();
  if (value === null || !Number.isFinite(value)) return null;
  // Compare the rounded figure so a tiny change never reads as a signed "−0%".
  const percent = Math.round(value * 100);
  const bad = worseWhenUp ? percent > 0 : percent < 0;
  const text = `${percent > 0 ? "+" : percent < 0 ? "−" : ""}${Math.abs(percent)}%`;
  return (
    <span>
      <span className={percent === 0 ? "" : bad ? "text-destructive" : "text-success"}>{text}</span>{" "}
      {ui("vs previous period")}
    </span>
  );
}

/** Failure-rate change in percentage points, neutral when it rounds to zero. */
function PointsDelta({ rate, previous }: { rate: number; previous: number }) {
  const ui = useUiTranslations();
  const points = Math.round((rate - previous) * 1000) / 10;
  const sign = points > 0 ? "+" : points < 0 ? "−" : "";
  return (
    <span className={points > 0 ? "text-destructive" : points < 0 ? "text-success" : ""}>
      {sign}
      {number(Math.abs(points))} {ui("pts")}
    </span>
  );
}

function Kpis({
  summary,
  previous,
  fleet,
}: {
  summary: TimelineSummary;
  previous: TimelineSummary;
  fleet: boolean;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const time = (at?: string | null) =>
    at
      ? new Intl.DateTimeFormat(displayLocale, {
          month: "short",
          day: "numeric",
          hour: "numeric",
          minute: "2-digit",
        }).format(new Date(at))
      : null;
  const rate = summary.failure_rate;
  const prevRate = previous.failure_rate;
  const tiles: { label: string; value: string; detail: React.ReactNode }[] = [
    {
      label: ui("Peak parallel sessions"),
      value: number(summary.peak_active),
      detail: summary.peak_at
        ? ui("at {value0}", { value0: time(summary.peak_at)! })
        : ui("No activity"),
    },
    {
      label: ui("Turns finished"),
      value: number(summary.turns_finished),
      detail: <Delta value={change(summary.turns_finished, previous.turns_finished)} />,
    },
    {
      label: ui("Failed turns"),
      value: rate == null ? "—" : `${number(rate * 100)}%`,
      detail:
        rate == null ? (
          ui("No finished turns")
        ) : (
          <span>
            {prevRate != null && <PointsDelta rate={rate} previous={prevRate} />}
            {prevRate != null && " · "}
            {ui("{value0} turns", { value0: number(summary.turns_failed) })}
          </span>
        ),
    },
    fleet && summary.seconds_above_90 != null
      ? {
          label: ui("Time above 90% load"),
          value: duration(summary.seconds_above_90 * 1000),
          detail: ui("worker slots in use"),
        }
      : {
          label: ui("Longest wait for a worker"),
          value: duration(summary.longest_wait_ms),
          detail: summary.longest_wait_at
            ? ui("at {value0}", { value0: time(summary.longest_wait_at)! })
            : "",
        },
    {
      label: ui("Model spend"),
      value: usd(summary.cost_usd),
      detail: <Delta value={change(summary.cost_usd, previous.cost_usd)} worseWhenUp />,
    },
  ];
  return (
    <div className="grid grid-cols-2 border bg-card md:grid-cols-5" data-testid="timeline-kpis">
      {tiles.map((t) => (
        <div key={t.label} className="border-b border-r p-4 last:border-r-0 md:border-b-0">
          <div className="text-[11px] text-muted-foreground">{t.label}</div>
          <div className="mt-1 text-2xl font-semibold tabular-nums">{t.value}</div>
          <div className="mt-0.5 text-[11px] text-muted-foreground">{t.detail}</div>
        </div>
      ))}
    </div>
  );
}

function WorkerStrip({
  data,
  formatX,
}: {
  data: MonitoringTimeline;
  formatX: (t: number) => string;
}) {
  const ui = useUiTranslations();
  const ids = [
    ...new Set(data.buckets.flatMap((b) => b.worker_loads.map((w) => w.worker_id))),
  ].sort();
  const n = data.buckets.length;
  return (
    <section className="border bg-card p-4">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">{ui("Worker load")}</h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {ui("Used slots divided by capacity per worker; red is above 90%.")}
          </p>
        </div>
        <ChartLegend
          series={[
            { key: "low", label: ui("Low"), color: "hsl(var(--info) / 0.25)" },
            { key: "high", label: ui("High"), color: "hsl(var(--info))" },
            { key: "full", label: ui("Saturated"), color: "hsl(var(--destructive))" },
          ]}
        />
      </div>
      {ids.length === 0 ? (
        <p className="text-xs text-muted-foreground">{ui("No worker reports in this range.")}</p>
      ) : (
        <div className="space-y-1.5">
          {ids.map((id) => (
            <div key={id} className="flex items-center gap-2">
              <span
                className="w-24 truncate text-right font-mono text-[10px] text-muted-foreground"
                title={id}
              >
                {id}
              </span>
              <svg
                viewBox={`0 0 ${n} 1`}
                preserveAspectRatio="none"
                className="h-4 flex-1"
                aria-label={id}
              >
                {data.buckets.map((b, i) => {
                  const w = b.worker_loads.find((x) => x.worker_id === id);
                  if (!w || !w.capacity) return null;
                  const u = Math.min(1, w.load_avg / w.capacity);
                  return (
                    <rect
                      key={b.start}
                      x={i}
                      y={0}
                      width={1.02}
                      height={1}
                      fill={u > 0.9 ? "hsl(var(--destructive))" : "hsl(var(--info))"}
                      fillOpacity={0.12 + 0.78 * u}
                    >
                      <title>{`${formatX(Date.parse(b.start))} · ${number(w.load_avg)}/${w.capacity}`}</title>
                    </rect>
                  );
                })}
              </svg>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function SelectionRail({
  scope,
  data,
  selection,
  onClear,
  onZoom,
  tooltipTime,
}: {
  scope: MonitoringScope;
  data: MonitoringTimeline;
  selection: TimeSelection | null;
  onClear: () => void;
  onZoom: (from: string, to: string) => void;
  tooltipTime: (t: number) => string;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const totals = selection ? selectionTotals(data, selection) : null;
  const from = totals?.from ?? data.from;
  const to = totals?.to ?? data.to;
  const windowFits = Date.parse(to) - Date.parse(from) <= 7 * 86400_000;
  const breakdown = useQuery({
    queryKey: ["monitoring-timeline-window", scope, from, to],
    queryFn: () => getMonitoringTimelineWindow(scope, from, to),
    enabled: windowFits,
    retry: false,
  });
  const single = selection && selection.start === selection.end;
  const fmt = new Intl.DateTimeFormat(displayLocale, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  const phaseColor = { model: C.model, tools: C.tools, waiting: C.waiting, runnable: C.runnable };
  const phaseLabel = {
    model: ui("Model"),
    tools: ui("Tools"),
    waiting: ui("Waiting"),
    runnable: ui("Runnable"),
  };
  return (
    <>
      <RailSection
        label={
          selection ? (single ? ui("Selected time") : ui("Selected range")) : ui("Whole range")
        }
      >
        {totals ? (
          <>
            <h2 className="text-sm font-semibold">
              {single
                ? tooltipTime(Date.parse(totals.from))
                : `${fmt.format(new Date(totals.from))} – ${fmt.format(new Date(totals.to))}`}
            </h2>
            <dl className="mt-3 space-y-2 text-xs">
              <Row label={ui("Peak parallel sessions")} value={number(totals.peak)} />
              <Row label={ui("Average parallel")} value={number(totals.average)} />
              <Row label={ui("Turns started")} value={number(totals.turnsStarted)} />
              <Row
                label={ui("Turns failed")}
                value={number(totals.turnsFailed)}
                warn={totals.turnsFailed > 0}
              />
              <Row label={ui("Tool calls")} value={number(totals.toolCalls)} />
              <Row label={ui("Longest wait for a worker")} value={duration(totals.longestWaitMs)} />
              <Row label={ui("Model spend")} value={usd(totals.spend)} />
            </dl>
            {!single && totals.phases.some((p) => p.share > 0) && (
              <>
                <h3 className="mt-4 border-t pt-3 text-xs font-semibold">
                  {ui("Most time spent in")}
                </h3>
                <dl className="mt-2 space-y-2 text-xs">
                  {totals.phases.map((p) => (
                    <div key={p.key} className="flex justify-between gap-3">
                      <dt className="text-muted-foreground">
                        <span
                          className="mr-2 inline-block size-2 rounded-full"
                          style={{ background: phaseColor[p.key] }}
                        />
                        {phaseLabel[p.key]}
                      </dt>
                      <dd className="tabular-nums">{Math.round(p.share * 100)}%</dd>
                    </div>
                  ))}
                </dl>
              </>
            )}
            <div className="mt-4 flex flex-col items-start gap-1">
              {!single && (
                <Button variant="ghost" size="sm" onClick={() => onZoom(totals.from, totals.to)}>
                  {ui("Zoom to selection →")}
                </Button>
              )}
              <Button variant="ghost" size="sm" onClick={onClear}>
                {ui("Clear selection")}
              </Button>
            </div>
          </>
        ) : (
          <p className="text-xs leading-relaxed text-muted-foreground">
            {ui(
              "Click a bucket on any chart to see its numbers. Drag across a chart to select a range.",
            )}
          </p>
        )}
      </RailSection>
      {windowFits && breakdown.data && (
        <RailSection label={scope === "fleet" ? ui("Busiest organizations") : ui("Busiest agents")}>
          {breakdown.data.busiest.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              {ui("No turns started in this window.")}
            </p>
          ) : (
            <dl className="space-y-2 text-xs">
              {breakdown.data.busiest.map((r) => (
                <div key={r.id ?? "none"} className="flex justify-between gap-3">
                  <dt className="truncate">
                    {scope === "fleet" ? (
                      <span className="font-mono text-[11px]">{r.id}</span>
                    ) : r.id ? (
                      <Link href={`/agents/${r.id}`} className="underline-offset-4 hover:underline">
                        {r.name}
                      </Link>
                    ) : (
                      <span className="text-muted-foreground">
                        {ui("Sessions without an agent")}
                      </span>
                    )}
                  </dt>
                  <dd className="shrink-0 tabular-nums text-muted-foreground">
                    {ui("{value0} sessions", { value0: number(r.sessions) })}
                  </dd>
                </div>
              ))}
            </dl>
          )}
          <h3 className="mt-4 border-t pt-3 text-xs font-semibold">
            {ui("Failed turns by cause")}
          </h3>
          {breakdown.data.failures.length === 0 ? (
            <p className="mt-2 text-xs text-muted-foreground">
              {ui("No failed turns in this window.")}
            </p>
          ) : (
            <dl className="mt-2 space-y-2 text-xs">
              {breakdown.data.failures.map((f) => (
                <div key={f.code} className="flex justify-between gap-3">
                  <dt>
                    <span className="mr-2 inline-block size-2 rounded-full bg-destructive" />
                    {f.code === "other" ? ui("other") : f.code.replaceAll("_", " ")}
                  </dt>
                  <dd className="tabular-nums">{number(f.turns)}</dd>
                </div>
              ))}
            </dl>
          )}
        </RailSection>
      )}
      {data.markers.length > 0 && scope === "fleet" && (
        <RailSection label={ui("Changes in this range")}>
          <dl className="space-y-2 text-xs">
            {data.markers.slice(-8).map((m) => (
              <div key={`${m.at}-${m.kind}-${m.label}`} className="flex justify-between gap-3">
                <dt className="text-muted-foreground">{fmt.format(new Date(m.at))}</dt>
                <dd className="truncate text-right">
                  {m.kind === "deploy"
                    ? ui("deploy {value0}", { value0: m.label })
                    : m.kind === "worker_joined"
                      ? ui("{value0} joined", { value0: m.label })
                      : ui("{value0} left", { value0: m.label })}
                </dd>
              </div>
            ))}
          </dl>
        </RailSection>
      )}
    </>
  );
}

function Row({ label, value, warn = false }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className="flex justify-between gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={`text-right tabular-nums ${warn ? "text-destructive" : ""}`}>{value}</dd>
    </div>
  );
}
