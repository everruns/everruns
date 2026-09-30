"use client";

import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { MiniTimeline, type TimelineBin } from "./mini-timeline";

export type StatStripStats = {
  health: string;
  healthSub: string;
  invocations24h: number;
  invocationSub: string;
  successRate: number | null;
  successSub: string;
  timeline: TimelineBin[];
};

export function StatStrip({
  stats,
  loading = false,
}: {
  stats: StatStripStats;
  loading?: boolean;
}) {
  const items = [
    { label: "Health", value: stats.health, sub: stats.healthSub },
    { label: "Invocations · 24h", value: String(stats.invocations24h), sub: stats.invocationSub },
    {
      label: "Success rate",
      value: stats.successRate === null ? "No runs" : `${stats.successRate.toFixed(1)}%`,
      sub: stats.successSub,
    },
  ];

  return (
    <div className="grid gap-3 lg:grid-cols-4">
      {items.map((item) => (
        <Card key={item.label}>
          <CardContent className="space-y-2 py-4">
            <p className="text-xs font-medium uppercase text-muted-foreground">{item.label}</p>
            {loading ? (
              <Skeleton className="h-7 w-24" />
            ) : (
              <p className="text-2xl font-semibold">{item.value}</p>
            )}
            {loading ? (
              <Skeleton className="h-4 w-32" />
            ) : (
              <p className="text-sm text-muted-foreground">{item.sub}</p>
            )}
          </CardContent>
        </Card>
      ))}
      <Card>
        <CardContent className="space-y-3 py-4">
          <p className="text-xs font-medium uppercase text-muted-foreground">Activity</p>
          {loading ? <Skeleton className="h-7 w-full" /> : <MiniTimeline runs={stats.timeline} />}
          <p className="text-sm text-muted-foreground">Last 24 hours</p>
        </CardContent>
      </Card>
    </div>
  );
}
