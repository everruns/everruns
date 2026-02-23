"use client";

// Durable metrics time-series charts
//
// Renders 4 charts from MetricsPoint[] data:
// 1. Workflow Status - running vs pending (stacked area)
// 2. Task Status - pending vs claimed (stacked area)
// 3. Throughput - completed/failed tasks per interval (line)
// 4. System Load - load %, active workers, DLQ size (line)

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

// Tailwind-compatible colors
const COLORS = {
  running: "#3b82f6", // blue-500
  pending: "#eab308", // yellow-500
  claimed: "#8b5cf6", // violet-500
  completed: "#22c55e", // green-500
  failed: "#ef4444", // red-500
  load: "#f97316", // orange-500
  workers: "#06b6d4", // cyan-500
  dlq: "#ef4444", // red-500
};

function formatTime(timestamp: string): string {
  const d = new Date(timestamp);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

interface ChartData {
  time: string;
  timestamp: string;
  running_workflows: number;
  pending_workflows: number;
  pending_tasks: number;
  claimed_tasks: number;
  load_percentage: number;
  active_workers: number;
  dlq_size: number;
  // Computed rates (delta per interval)
  completed_rate: number;
  failed_rate: number;
}

function computeChartData(points: MetricsPoint[]): ChartData[] {
  return points.map((p, i) => {
    const prev = i > 0 ? points[i - 1] : null;
    const completedDelta = prev
      ? Math.max(0, p.tasks_completed_total - prev.tasks_completed_total)
      : 0;
    const failedDelta = prev ? Math.max(0, p.tasks_failed_total - prev.tasks_failed_total) : 0;

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
        <CardDescription className="text-xs">Running vs pending over time</CardDescription>
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
              dataKey="running_workflows"
              name="Running"
              stackId="1"
              stroke={COLORS.running}
              fill={COLORS.running}
              fillOpacity={0.3}
            />
            <Area
              type="monotone"
              dataKey="pending_workflows"
              name="Pending"
              stackId="1"
              stroke={COLORS.pending}
              fill={COLORS.pending}
              fillOpacity={0.3}
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
        <CardDescription className="text-xs">Pending vs claimed over time</CardDescription>
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
              dataKey="claimed_tasks"
              name="Claimed"
              stackId="1"
              stroke={COLORS.claimed}
              fill={COLORS.claimed}
              fillOpacity={0.3}
            />
            <Area
              type="monotone"
              dataKey="pending_tasks"
              name="Pending"
              stackId="1"
              stroke={COLORS.pending}
              fill={COLORS.pending}
              fillOpacity={0.3}
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
          Completed and failed tasks per interval
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
        <CardDescription className="text-xs">Load %, workers, and DLQ size</CardDescription>
      </CardHeader>
      <CardContent className="pb-2">
        <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
          <LineChart data={data}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
            <XAxis {...xAxisProps(data)} />
            <YAxis {...yAxisDefaults} />
            <Tooltip content={<ChartTooltip />} />
            <Line
              type="monotone"
              dataKey="load_percentage"
              name="Load %"
              stroke={COLORS.load}
              strokeWidth={2}
              dot={false}
            />
            <Line
              type="monotone"
              dataKey="active_workers"
              name="Workers"
              stroke={COLORS.workers}
              strokeWidth={2}
              dot={false}
            />
            <Line
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
 * Renders empty state if no data points yet.
 */
export function MetricsCharts({ points }: { points: MetricsPoint[] }) {
  const chartData = useMemo(() => computeChartData(points), [points]);

  if (chartData.length < 2) {
    return (
      <Card>
        <CardContent className="flex items-center justify-center py-8">
          <p className="text-sm text-muted-foreground">
            Collecting metrics data... Charts will appear after a few data points are recorded.
          </p>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <WorkflowStatusChart data={chartData} />
      <TaskStatusChart data={chartData} />
      <ThroughputChart data={chartData} />
      <SystemLoadChart data={chartData} />
    </div>
  );
}
