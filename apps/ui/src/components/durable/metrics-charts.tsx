"use client";

// Durable metrics time-series charts
//
// Design decision: show RATES from cumulative counters, not point-in-time gauges.
// Running/pending gauges are always 0 when workflows complete faster than the
// 10-second sampling interval. Rates (started/completed/failed per interval)
// give meaningful throughput visibility regardless of workflow duration.
//
// Zero-backfill: gauge fields are zero-filled for empty slots; cumulative counter
// fields are carry-forwarded from the last known value to avoid false rate spikes.

import { useMemo } from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import type { MetricsPoint } from "@/lib/api/types";

/** Chart time window in minutes */
const WINDOW_MINUTES = 15;
/** Backend sampling resolution in seconds */
const RESOLUTION_SECONDS = 10;
/** Number of data points in the window */
const WINDOW_SLOTS = (WINDOW_MINUTES * 60) / RESOLUTION_SECONDS; // 90

// Tailwind-compatible colors
const COLORS = {
  started: "#3b82f6", // blue-500
  pending: "#eab308", // yellow-500
  claimed: "#8b5cf6", // violet-500
  completed: "#22c55e", // green-500
  failed: "#ef4444", // red-500
  load: "#f97316", // orange-500
  workers: "#06b6d4", // cyan-500
  dlq: "#ef4444", // red-500
  running: "#3b82f6", // blue-500
};

function formatTime(timestamp: string): string {
  const d = new Date(timestamp);
  return d.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

interface ChartData {
  time: string;
  timestamp: string;
  // Gauges (point-in-time)
  running_workflows: number;
  pending_workflows: number;
  pending_tasks: number;
  claimed_tasks: number;
  load_percentage: number;
  active_workers: number;
  dlq_size: number;
  // Computed rates (delta per interval) from cumulative counters
  completed_rate: number;
  failed_rate: number;
  started_rate: number;
  workflow_completed_rate: number;
  workflow_failed_rate: number;
  workflow_started_rate: number;
}

/** Cumulative counter field names in MetricsPoint (all are numeric) */
const COUNTER_FIELDS = [
  "tasks_completed_total",
  "tasks_failed_total",
  "tasks_started_total",
  "workflows_completed_total",
  "workflows_failed_total",
  "workflows_started_total",
] as const;

type CounterField = (typeof COUNTER_FIELDS)[number];

/** Get a counter value from a MetricsPoint */
function getCounter(p: MetricsPoint, field: CounterField): number {
  return p[field];
}

/** Set a counter value on a mutable metrics point */
function setCounter(p: MetricsPoint, field: CounterField, value: number): void {
  // All CounterField keys are valid MetricsPoint numeric fields.
  // Use indexed access on a type-safe mapped helper to avoid double cast.
  const mutable: { -readonly [K in CounterField]: number } = p;
  mutable[field] = value;
}

/** Create a zero-valued MetricsPoint at a given timestamp */
function zeroPoint(timestamp: string): MetricsPoint {
  return {
    timestamp,
    running_workflows: 0,
    pending_workflows: 0,
    pending_tasks: 0,
    claimed_tasks: 0,
    active_workers: 0,
    load_percentage: 0,
    dlq_size: 0,
    tasks_completed_total: 0,
    tasks_failed_total: 0,
    tasks_started_total: 0,
    workflows_completed_total: 0,
    workflows_failed_total: 0,
    workflows_started_total: 0,
  };
}

/**
 * Build a fixed 15-minute window of data points at RESOLUTION_SECONDS intervals.
 * Actual data is snapped to the nearest slot; missing slots are filled:
 * - Gauge fields: zero-filled (no activity at that moment)
 * - Counter fields: carry-forwarded (last known cumulative value, prevents false rate spikes)
 */
function buildTimeWindow(points: MetricsPoint[]): MetricsPoint[] {
  const now = Date.now();
  const windowStart = now - WINDOW_MINUTES * 60 * 1000;
  const slotMs = RESOLUTION_SECONDS * 1000;

  // Initialize all slots with zeros
  const slots: MetricsPoint[] = [];
  for (let i = 0; i < WINDOW_SLOTS; i++) {
    const t = new Date(windowStart + i * slotMs);
    slots[i] = zeroPoint(t.toISOString());
  }

  // Track which slots have real data
  const hasData: boolean[] = new Array(WINDOW_SLOTS).fill(false);

  // Overlay actual data onto nearest slots
  for (const point of points) {
    const pointMs = new Date(point.timestamp).getTime();
    if (pointMs < windowStart) continue;
    const slotIndex = Math.round((pointMs - windowStart) / slotMs);
    if (slotIndex >= 0 && slotIndex < WINDOW_SLOTS) {
      slots[slotIndex] = point;
      hasData[slotIndex] = true;
    }
  }

  // Carry-forward cumulative counters through empty slots.
  // This prevents rate spikes at boundaries between zero-filled and real data.
  let lastCounters: Partial<Record<CounterField, number>> = {};
  for (let i = 0; i < WINDOW_SLOTS; i++) {
    if (hasData[i]) {
      // Update last known counter values from real data
      for (const field of COUNTER_FIELDS) {
        lastCounters[field] = getCounter(slots[i], field);
      }
    } else if (Object.keys(lastCounters).length > 0) {
      // Fill empty slot with last known counter values
      const slot = slots[i];
      for (const field of COUNTER_FIELDS) {
        if (field in lastCounters) {
          setCounter(slot, field, lastCounters[field]!);
        }
      }
    }
  }

  // Also backfill counters backward for slots before the first real data point.
  // Find the first real data point's counter values and fill backwards.
  const firstRealIdx = hasData.indexOf(true);
  if (firstRealIdx > 0) {
    const firstReal = slots[firstRealIdx];
    for (let i = 0; i < firstRealIdx; i++) {
      const slot = slots[i];
      for (const field of COUNTER_FIELDS) {
        setCounter(slot, field, getCounter(firstReal, field));
      }
    }
  }

  return slots;
}

function computeChartData(points: MetricsPoint[]): ChartData[] {
  const windowed = buildTimeWindow(points);

  return windowed.map((p, i) => {
    const prev = i > 0 ? windowed[i - 1] : null;
    const completedDelta = prev
      ? Math.max(0, p.tasks_completed_total - prev.tasks_completed_total)
      : 0;
    const failedDelta = prev ? Math.max(0, p.tasks_failed_total - prev.tasks_failed_total) : 0;
    const startedDelta = prev ? Math.max(0, p.tasks_started_total - prev.tasks_started_total) : 0;
    const wfCompletedDelta = prev
      ? Math.max(0, p.workflows_completed_total - prev.workflows_completed_total)
      : 0;
    const wfFailedDelta = prev
      ? Math.max(0, p.workflows_failed_total - prev.workflows_failed_total)
      : 0;
    const wfStartedDelta = prev
      ? Math.max(0, p.workflows_started_total - prev.workflows_started_total)
      : 0;

    return {
      time: formatTime(p.timestamp),
      timestamp: p.timestamp,
      running_workflows: p.running_workflows,
      pending_workflows: p.pending_workflows,
      pending_tasks: p.pending_tasks,
      claimed_tasks: p.claimed_tasks,
      load_percentage: Math.round(p.load_percentage * 10) / 10,
      active_workers: p.active_workers,
      dlq_size: p.dlq_size,
      completed_rate: completedDelta,
      failed_rate: failedDelta,
      started_rate: startedDelta,
      workflow_completed_rate: wfCompletedDelta,
      workflow_failed_rate: wfFailedDelta,
      workflow_started_rate: wfStartedDelta,
    };
  });
}

// Custom tooltip component
function ChartTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: Array<{ name: string; value: number; color: string }>;
  label?: string;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="rounded-lg border bg-background p-2 shadow-sm">
      <p className="text-xs text-muted-foreground mb-1">{label}</p>
      {payload.map((entry) => (
        <div key={entry.name} className="flex items-center gap-2 text-xs">
          <div className="w-2 h-2 rounded-full" style={{ backgroundColor: entry.color }} />
          <span className="text-muted-foreground">{entry.name}:</span>
          <span className="font-medium">{entry.value}</span>
        </div>
      ))}
    </div>
  );
}

// Shared chart config
const CHART_HEIGHT = 200;
const TICK_STYLE = { fontSize: 10, fill: "hsl(var(--muted-foreground))" };

function xAxisProps(data: ChartData[]) {
  // Show ~6 ticks evenly spaced
  const tickCount = Math.min(6, data.length);
  const interval = tickCount > 0 ? Math.floor(data.length / tickCount) : 0;
  return {
    dataKey: "time" as const,
    tick: TICK_STYLE,
    tickLine: false,
    axisLine: false,
    interval: Math.max(interval, 1),
  };
}

const yAxisDefaults = {
  tick: TICK_STYLE,
  tickLine: false,
  axisLine: false,
  width: 40,
};

export function WorkflowStatusChart({ data }: { data: ChartData[] }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium">Workflows</CardTitle>
        <CardDescription className="text-xs">
          Throughput per interval: started, completed, failed (last 15 min)
        </CardDescription>
      </CardHeader>
      <CardContent className="pb-2">
        <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
          <AreaChart data={data}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
            <XAxis {...xAxisProps(data)} />
            <YAxis {...yAxisDefaults} allowDecimals={false} />
            <Tooltip content={<ChartTooltip />} />
            <Area
              type="monotone"
              dataKey="workflow_started_rate"
              name="Started"
              stackId="1"
              stroke={COLORS.started}
              fill={COLORS.started}
              fillOpacity={0.3}
            />
            <Line
              type="monotone"
              dataKey="workflow_completed_rate"
              name="Completed"
              stroke={COLORS.completed}
              strokeWidth={2}
              dot={false}
            />
            <Line
              type="monotone"
              dataKey="workflow_failed_rate"
              name="Failed"
              stroke={COLORS.failed}
              strokeWidth={2}
              dot={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </CardContent>
    </Card>
  );
}

export function TaskStatusChart({ data }: { data: ChartData[] }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium">Tasks</CardTitle>
        <CardDescription className="text-xs">
          Throughput per interval: started, completed, failed (last 15 min)
        </CardDescription>
      </CardHeader>
      <CardContent className="pb-2">
        <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
          <AreaChart data={data}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
            <XAxis {...xAxisProps(data)} />
            <YAxis {...yAxisDefaults} allowDecimals={false} />
            <Tooltip content={<ChartTooltip />} />
            <Area
              type="monotone"
              dataKey="started_rate"
              name="Started"
              stackId="1"
              stroke={COLORS.started}
              fill={COLORS.started}
              fillOpacity={0.3}
            />
            <Line
              type="monotone"
              dataKey="completed_rate"
              name="Completed"
              stroke={COLORS.completed}
              strokeWidth={2}
              dot={false}
            />
            <Line
              type="monotone"
              dataKey="failed_rate"
              name="Failed"
              stroke={COLORS.failed}
              strokeWidth={2}
              dot={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </CardContent>
    </Card>
  );
}

export function ThroughputChart({ data }: { data: ChartData[] }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium">Throughput</CardTitle>
        <CardDescription className="text-xs">
          Task completions and failures per interval (last 15 min)
        </CardDescription>
      </CardHeader>
      <CardContent className="pb-2">
        <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
          <LineChart data={data}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
            <XAxis {...xAxisProps(data)} />
            <YAxis {...yAxisDefaults} allowDecimals={false} />
            <Tooltip content={<ChartTooltip />} />
            <Line
              type="monotone"
              dataKey="completed_rate"
              name="Completed"
              stroke={COLORS.completed}
              strokeWidth={2}
              dot={false}
            />
            <Line
              type="monotone"
              dataKey="failed_rate"
              name="Failed"
              stroke={COLORS.failed}
              strokeWidth={2}
              dot={false}
            />
          </LineChart>
        </ResponsiveContainer>
      </CardContent>
    </Card>
  );
}

export function SystemLoadChart({ data }: { data: ChartData[] }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium">System Load</CardTitle>
        <CardDescription className="text-xs">
          Load %, workers, and DLQ size (last 15 min)
        </CardDescription>
      </CardHeader>
      <CardContent className="pb-2">
        <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
          <LineChart data={data}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
            <XAxis {...xAxisProps(data)} />
            <YAxis yAxisId="pct" {...yAxisDefaults} domain={[0, 100]} />
            <YAxis yAxisId="count" {...yAxisDefaults} orientation="right" width={30} />
            <Tooltip content={<ChartTooltip />} />
            <Line
              yAxisId="pct"
              type="monotone"
              dataKey="load_percentage"
              name="Load %"
              stroke={COLORS.load}
              strokeWidth={2}
              dot={false}
            />
            <Line
              yAxisId="count"
              type="monotone"
              dataKey="active_workers"
              name="Workers"
              stroke={COLORS.workers}
              strokeWidth={2}
              dot={false}
            />
            <Line
              yAxisId="count"
              type="monotone"
              dataKey="dlq_size"
              name="DLQ"
              stroke={COLORS.dlq}
              strokeWidth={2}
              dot={false}
              strokeDasharray="5 5"
            />
          </LineChart>
        </ResponsiveContainer>
      </CardContent>
    </Card>
  );
}

/**
 * Full metrics dashboard with all 4 charts in a 2x2 grid.
 * Always shows a 15-minute window, zero-filled when data is missing.
 */
export function MetricsCharts({ points }: { points: MetricsPoint[] }) {
  const chartData = useMemo(() => computeChartData(points), [points]);

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <WorkflowStatusChart data={chartData} />
      <TaskStatusChart data={chartData} />
      <ThroughputChart data={chartData} />
      <SystemLoadChart data={chartData} />
    </div>
  );
}
