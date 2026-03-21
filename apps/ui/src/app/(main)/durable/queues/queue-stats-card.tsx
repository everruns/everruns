"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ActivityTypeStats } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { formatDurationCompact } from "@/lib/formatting";

export function QueueStatsCard({
  activityType,
  stats,
}: {
  activityType: string;
  stats: ActivityTypeStats;
}) {
  const totalActive = stats.pending + stats.claimed;
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium truncate">{activityType}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="grid grid-cols-2 gap-3 text-sm">
          <div>
            <p className="text-muted-foreground text-xs">Pending</p>
            <p className="font-semibold text-lg">{stats.pending}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Claimed</p>
            <p className="font-semibold text-lg">{stats.claimed}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Completed/hr</p>
            <p className="font-medium">{stats.completed_last_hour}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Failed/hr</p>
            <p className={cn("font-medium", stats.failed_last_hour > 0 && "text-red-500")}>
              {stats.failed_last_hour}
            </p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Avg Duration</p>
            <p className="font-medium">{formatDurationCompact(stats.avg_duration_ms)}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">p99 Duration</p>
            <p className="font-medium">{formatDurationCompact(stats.p99_duration_ms)}</p>
          </div>
        </div>
        {totalActive > 0 && (
          <div className="mt-3">
            <div className="h-1.5 w-full bg-muted rounded-full overflow-hidden">
              <div
                className="h-full bg-blue-500 rounded-full"
                style={{ width: `${(stats.claimed / totalActive) * 100}%` }}
              />
            </div>
            <p className="text-xs text-muted-foreground mt-1">
              {stats.claimed}/{totalActive} processing
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
