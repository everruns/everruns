"use client";

import { useQuery } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { listAppRuns } from "@/lib/api/apps";
import { cn } from "@/lib/utils";

function relativeTime(value?: string | null): string {
  if (!value) return "Unknown";
  const seconds = Math.round((new Date(value).getTime() - Date.now()) / 1000);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 60) return formatter.format(seconds, "second");
  if (abs < 3600) return formatter.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return formatter.format(Math.round(seconds / 3600), "hour");
  return formatter.format(Math.round(seconds / 86400), "day");
}

export function LiveActivityRail({ appId, className }: { appId: string; className?: string }) {
  const polling = useQuery({
    queryKey: ["apps", appId, "runs"],
    queryFn: () => listAppRuns(appId),
    staleTime: 5000,
    // Poll fast (5s) only while a run is in flight; when everything is in a
    // terminal state (completed/failed/skipped) fall back to a slow 30s poll so
    // newly-started runs still surface without the unconditional 5s churn.
    refetchInterval: (query) => {
      const runs = query.state.data?.data ?? [];
      const hasInFlight = runs.some((run) => run.status === "pending" || run.status === "running");
      return hasInFlight ? 5000 : 30000;
    },
  });

  const visibleEvents = polling.data?.data ?? [];

  return (
    <Card className={cn("h-fit", className)}>
      <CardHeader>
        <CardTitle>Live activity</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {polling.isError ? (
          <div className="border border-destructive/40 p-4 text-sm text-destructive">
            Unable to load run history.
          </div>
        ) : visibleEvents.length === 0 ? (
          <div className="border border-dashed p-4 text-sm text-muted-foreground">
            Runs will appear here as channels invoke this app.
          </div>
        ) : (
          visibleEvents.slice(0, 12).map((event) => (
            <div key={event.id} className="border p-3">
              <div className="flex items-center justify-between gap-3">
                <p className="truncate text-sm font-medium">
                  {event.channel_name ?? event.channel_id}
                </p>
                <Badge variant={event.status === "failed" ? "destructive" : "secondary"}>
                  {event.status}
                </Badge>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                {event.channel_type} · {relativeTime(event.completed_at ?? event.created_at)}
              </p>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}
