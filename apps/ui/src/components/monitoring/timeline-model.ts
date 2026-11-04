import type { MonitoringTimeline, TimelineBucket, TimelineSummary } from "@/lib/api/types";
import type { TimeRow, TimeSelection } from "@/components/charts/time-series-chart";

// Phase colors match Execution Flow (model.ts).
export const timelineColors = {
  model: "hsl(var(--info))",
  tools: "hsl(var(--success))",
  waiting: "hsl(var(--muted-foreground))",
  runnable: "hsl(var(--accent))",
  setup: "hsl(var(--border))",
  rebuilt: "hsl(var(--muted-foreground))",
  peak: "hsl(var(--primary))",
  capacity: "hsl(var(--muted-foreground))",
  completed: "hsl(var(--success))",
  cancelled: "hsl(var(--muted-foreground))",
  failed: "hsl(var(--destructive))",
  wait: "hsl(var(--warning))",
  spend: "hsl(var(--primary))",
  latency: "hsl(var(--info))",
};

export const timelineRanges = ["15m", "1h", "6h", "24h", "7d", "30d"] as const;

/** Chart rows. History rebuilt from turn records has no phase split, so it is drawn as one area. */
export function timelineRows(buckets: TimelineBucket[]): TimeRow[] {
  return buckets.map((b) => {
    const rebuilt = b.backfilled && !b.sampled;
    return {
      t: Date.parse(b.start),
      model: rebuilt ? 0 : b.model_avg,
      tools: rebuilt ? 0 : b.tools_avg,
      waiting: rebuilt ? 0 : b.waiting_avg,
      runnable: rebuilt ? 0 : b.runnable_avg,
      setup: rebuilt ? 0 : b.setup_avg,
      rebuilt: rebuilt ? b.active_avg : 0,
      peak: b.active_peak,
      capacity: b.worker_capacity ?? null,
      load: b.worker_load_avg ?? null,
      completed: b.turns_completed,
      cancelled: b.turns_cancelled,
      failed: b.turns_failed,
      wait: b.oldest_runnable_ms == null ? null : b.oldest_runnable_ms / 1000,
      tool_calls: b.tool_calls,
      tool_failures: b.tool_failures,
      errors: b.turns_failed + b.tool_failures,
      spend: b.cost_usd,
      latency: b.model_latency_p95_ms == null ? null : b.model_latency_p95_ms / 1000,
      takeovers: b.lease_takeovers,
    };
  });
}

export interface SelectionTotals {
  from: string;
  to: string;
  peak: number;
  average: number;
  turnsStarted: number;
  turnsFailed: number;
  toolCalls: number;
  longestWaitMs: number | null;
  spend: number;
  phases: { key: "model" | "tools" | "waiting" | "runnable"; share: number }[];
}

/** Totals for a bucket range, read from the buckets already loaded. */
export function selectionTotals(
  data: MonitoringTimeline,
  selection: TimeSelection,
): SelectionTotals {
  const picked = data.buckets.slice(selection.start, selection.end + 1);
  const sum = (f: (b: TimelineBucket) => number) => picked.reduce((a, b) => a + f(b), 0);
  const phaseSum = {
    model: sum((b) => b.model_avg),
    tools: sum((b) => b.tools_avg),
    waiting: sum((b) => b.waiting_avg),
    runnable: sum((b) => b.runnable_avg),
  };
  const phaseTotal = Object.values(phaseSum).reduce((a, b) => a + b, 0);
  const waits = picked.map((b) => b.oldest_runnable_ms).filter((v): v is number => v != null);
  const last = picked.at(-1);
  return {
    from: picked[0]?.start ?? data.from,
    to: last
      ? new Date(Date.parse(last.start) + data.bucket_seconds * 1000).toISOString()
      : data.to,
    peak: Math.max(0, ...picked.map((b) => b.active_peak)),
    average: picked.length ? sum((b) => b.active_avg) / picked.length : 0,
    turnsStarted: sum((b) => b.turns_started),
    turnsFailed: sum((b) => b.turns_failed),
    toolCalls: sum((b) => b.tool_calls),
    longestWaitMs: waits.length ? Math.max(...waits) : null,
    spend: sum((b) => b.cost_usd),
    phases: (Object.keys(phaseSum) as (keyof typeof phaseSum)[]).map((key) => ({
      key,
      share: phaseTotal > 0 ? phaseSum[key] / phaseTotal : 0,
    })),
  };
}

/** Relative change against the previous period, or null when there is nothing to compare. */
export function change(current: number, previous: number): number | null {
  if (!previous) return null;
  return (current - previous) / previous;
}

export function failureRate(s: TimelineSummary): number | null {
  return s.failure_rate ?? null;
}

/** Ranges up to a day label times, longer ones label dates. */
export function axisFormatter(bucketSeconds: number, locale: string) {
  const long = bucketSeconds >= 3600;
  const time = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" });
  const date = new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" });
  return (t: number) => (long ? date : time).format(new Date(t));
}

export function bucketLabel(locale: string, bucketSeconds: number) {
  const full = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  const time = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" });
  return (t: number) =>
    `${full.format(new Date(t))} – ${time.format(new Date(t + bucketSeconds * 1000))}`;
}

export function usd(value: number): string {
  return new Intl.NumberFormat("en", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: value < 1 ? 3 : 2,
  }).format(value);
}
