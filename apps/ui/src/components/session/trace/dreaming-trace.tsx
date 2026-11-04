"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useQuery } from "@tanstack/react-query";
import { Moon } from "lucide-react";
import { listEvents } from "@/lib/api/events";

import type { DreamingActivityData as Activity } from "@/lib/api/types";

/** A separate bounded background lane: never included in foreground turn duration.
 * THREAT[TM-LLM-053]: Render only activity metadata, never notes or evidence.
 */
export function DreamingTrace({ sessionId }: { sessionId: string }) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const { data, error } = useQuery({
    queryKey: ["dreaming-trace", sessionId],
    queryFn: () => listEvents(sessionId, { types: ["dreaming.activity"], limit: 20 }),
    refetchInterval: 5000,
  });
  if (error)
    return (
      <p role="status" className="border-b px-6 py-2 text-xs text-muted-foreground">
        {ui("Background dreaming activity could not be loaded.")}
      </p>
    );
  if (!data?.length) return null;
  const latest = new Map<string, { activity: Activity; timestamp: string; started?: string }>();
  for (const event of data) {
    const activity = event.data as unknown as Activity;
    if (activity.operation_id) {
      const previous = latest.get(activity.operation_id);
      latest.set(activity.operation_id, {
        activity: {
          ...previous?.activity,
          ...activity,
          usage: activity.usage ?? previous?.activity.usage,
          model: activity.model ?? previous?.activity.model,
          cost_usd: activity.cost_usd ?? previous?.activity.cost_usd,
        },
        timestamp: event.ts,
        started:
          previous?.started ??
          (["queued", "started"].includes(activity.status) ? event.ts : undefined),
      });
    }
  }
  return (
    <details className="max-h-64 shrink-0 overflow-y-auto border-b px-6 py-2 text-xs">
      <summary className="flex cursor-pointer items-center gap-2 font-medium">
        <Moon className="h-3.5 w-3.5" />
        {ui("Background dreaming ·")} {latest.size} {ui("recent")}{" "}
        {latest.size === 1 ? ui("operation") : ui("operations")}
      </summary>
      <ul className="mt-2 space-y-2">
        {[...latest.values()].reverse().map(({ activity, timestamp, started }) => (
          <li key={activity.operation_id} className="rounded border px-3 py-2">
            <div className="flex flex-wrap gap-3">
              <span>{activity.status}</span>
              <span>
                {activity.note_count} {activity.note_count === 1 ? ui("note") : ui("notes")}
              </span>
              <time className="text-muted-foreground">
                {new Date(timestamp).toLocaleTimeString(displayLocale)}
              </time>
            </div>
            {started && (
              <p className="mt-1 text-muted-foreground">
                {ui("Elapsed:")}{" "}
                {Math.max(0, (Date.parse(timestamp) - Date.parse(started)) / 1000).toFixed(1)}s
              </p>
            )}
            {activity.usage && (
              <p className="mt-1 text-muted-foreground">
                {ui("Latest generation:")} {activity.usage.input_tokens} {ui("uncached input ·")}{" "}
                {activity.usage.cache_read_tokens ?? 0} {ui("cache read ·")}{" "}
                {activity.usage.cache_creation_tokens ?? 0} {ui("cache write ·")}{" "}
                {activity.usage.output_tokens} {ui("output")}
                {activity.usage.actual_cost_usd != null
                  ? ui(" · Reported cost: ${value0}", {
                      value0: activity.usage.actual_cost_usd.toFixed(4),
                    })
                  : activity.usage.estimated_cost_usd != null
                    ? ` · Estimated cost: $${activity.usage.estimated_cost_usd.toFixed(4)}`
                    : ""}
              </p>
            )}
            {activity.model && <p className="mt-1 text-muted-foreground">{activity.model}</p>}
            {activity.cost_usd != null && (
              <p className="mt-1 text-muted-foreground">
                {ui("Latest call reserved maximum: $")}
                {activity.cost_usd.toFixed(4)}
              </p>
            )}
            {activity.reason && <p className="mt-1 text-muted-foreground">{activity.reason}</p>}
            <p className="mt-1 font-mono text-muted-foreground">{activity.operation_id}</p>
          </li>
        ))}
      </ul>
    </details>
  );
}
