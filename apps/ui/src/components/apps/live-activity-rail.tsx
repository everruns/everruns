"use client";

import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { listAppRuns } from "@/lib/api/apps";
import type { AppRunEvent } from "@/lib/api/types";
import { createEventStream } from "@/lib/event-stream";
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
  const [events, setEvents] = useState<AppRunEvent[]>([]);
  const [streamUnavailable, setStreamUnavailable] = useState(false);

  const polling = useQuery({
    queryKey: ["apps", appId, "runs", streamUnavailable ? "polling" : "idle"],
    queryFn: () => listAppRuns(appId),
    enabled: streamUnavailable,
    refetchInterval: 5000,
  });

  useEffect(() => {
    setEvents([]);
    setStreamUnavailable(false);

    let source: ReturnType<typeof createEventStream>;
    try {
      source = createEventStream(`/api/v1/apps/${appId}/runs/stream`, { withCredentials: true });
    } catch {
      setStreamUnavailable(true);
      return;
    }
    source.addEventListener("run", (event) => {
      try {
        const next = JSON.parse((event as MessageEvent).data) as AppRunEvent;
        setEvents((existing) =>
          [next, ...existing.filter((item) => item.id !== next.id)].slice(0, 20),
        );
      } catch {
        setStreamUnavailable(true);
      }
    });
    source.onerror = () => {
      setStreamUnavailable(true);
      source.close();
    };

    return () => source.close();
  }, [appId]);

  const visibleEvents = useMemo(() => {
    const polled = polling.data?.data ?? [];
    return [...events, ...polled].filter(
      (event, index, all) => all.findIndex((candidate) => candidate.id === event.id) === index,
    );
  }, [events, polling.data]);

  return (
    <Card className={cn("h-fit", className)}>
      <CardHeader>
        <CardTitle>Live activity</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {visibleEvents.length === 0 ? (
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
        {streamUnavailable && (
          <p className="text-xs text-muted-foreground">
            Streaming unavailable; polling every 5 seconds.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
