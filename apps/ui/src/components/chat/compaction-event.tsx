"use client";

import type { Event } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";
import { CompactionDivider } from "./compaction-divider";
import { CompactionNotice } from "./compaction-notice";

export function isCompactionEvent(event: Event) {
  return [
    "context.compacting",
    "context.compacted",
    "context.compaction.skipped",
    "context.compaction.failed",
  ].includes(event.type);
}

export function latestCompactionEventId(events: readonly Event[]) {
  return events.findLast(isCompactionEvent)?.id;
}

export function renderCompactionEvent(event: Event, latestEventId?: string) {
  if (event.type === "context.compaction.failed") return <CompactionNotice key={event.id} failed />;
  if (event.type === "context.compacting")
    return event.id === latestEventId ? <CompactionNotice key={event.id} /> : null;
  const data = getEventData(event, "context.compacted");
  return data ? <CompactionDivider key={event.id} data={data} /> : null;
}
