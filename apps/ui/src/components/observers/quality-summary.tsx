"use client";

// Client-side quality aggregation for the observer detail "Quality" tab.
//
// Trends are computed from recent sampled scores only; full server-side
// aggregation is a follow-up. We group completed scores by scorer_key for the
// summary cards and by day for the trend chart.

import { useMemo } from "react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { TraceScore } from "@/lib/api/types";

export interface ScorerSummary {
  scorerKey: string;
  scored: number;
  passRate: number | null;
  avgValue: number | null;
}

export function summarizeScores(scores: TraceScore[]): ScorerSummary[] {
  const byKey = new Map<string, TraceScore[]>();
  for (const score of scores) {
    if (score.status !== "completed") continue;
    const list = byKey.get(score.scorer_key) ?? [];
    list.push(score);
    byKey.set(score.scorer_key, list);
  }

  return [...byKey.entries()]
    .map(([scorerKey, list]) => {
      const passable = list.filter((s) => s.pass !== undefined);
      const valued = list.filter((s) => s.value !== undefined);
      const passRate = passable.length
        ? passable.filter((s) => s.pass).length / passable.length
        : null;
      const avgValue = valued.length
        ? valued.reduce((sum, s) => sum + (s.value ?? 0), 0) / valued.length
        : null;
      return { scorerKey, scored: list.length, passRate, avgValue };
    })
    .sort((a, b) => a.scorerKey.localeCompare(b.scorerKey));
}

interface TrendPoint {
  day: string;
  passRate: number;
  count: number;
}

function buildTrend(scores: TraceScore[]): TrendPoint[] {
  const byDay = new Map<string, { passed: number; total: number }>();
  for (const score of scores) {
    if (score.status !== "completed" || score.pass === undefined) continue;
    const day = score.created_at.slice(0, 10);
    const entry = byDay.get(day) ?? { passed: 0, total: 0 };
    entry.total += 1;
    if (score.pass) entry.passed += 1;
    byDay.set(day, entry);
  }

  return [...byDay.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([day, { passed, total }]) => ({
      day,
      passRate: Math.round((passed / total) * 100),
      count: total,
    }));
}

function passRateColor(rate: number): string {
  if (rate >= 0.9) return "text-green-600";
  if (rate >= 0.7) return "text-yellow-600";
  return "text-red-600";
}

export function QualitySummary({ scores }: { scores: TraceScore[] }) {
  const summaries = useMemo(() => summarizeScores(scores), [scores]);
  const trend = useMemo(() => buildTrend(scores), [scores]);

  return (
    <div className="space-y-6">
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        {summaries.map((summary) => (
          <Card key={summary.scorerKey}>
            <CardHeader className="pb-2">
              <CardTitle className="font-mono text-sm">{summary.scorerKey}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-1 text-sm">
              <div className="flex justify-between">
                <span className="text-muted-foreground">Scored</span>
                <span>{summary.scored}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">Pass rate</span>
                <span
                  className={
                    summary.passRate !== null ? passRateColor(summary.passRate) : undefined
                  }
                >
                  {summary.passRate !== null ? `${Math.round(summary.passRate * 100)}%` : "—"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">Avg value</span>
                <span>{summary.avgValue !== null ? summary.avgValue.toFixed(2) : "—"}</span>
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      {trend.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium">Pass rate trend</CardTitle>
          </CardHeader>
          <CardContent className="pb-2">
            <ResponsiveContainer width="100%" height={220}>
              <LineChart data={trend}>
                <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
                <XAxis
                  dataKey="day"
                  tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
                  tickLine={false}
                  axisLine={false}
                />
                <YAxis
                  domain={[0, 100]}
                  tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
                  tickLine={false}
                  axisLine={false}
                  width={40}
                />
                <Tooltip />
                <Line
                  type="monotone"
                  dataKey="passRate"
                  name="Pass rate %"
                  stroke="#22c55e"
                  strokeWidth={2}
                  dot={false}
                />
              </LineChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
