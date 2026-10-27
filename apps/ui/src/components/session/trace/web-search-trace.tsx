"use client";

// Trace inspector section for a `web_search` step: which provider answered
// and one line per attempt (a fallback shows the failed attempt in red), with
// the details and raw-response drawer one click away. The step's events carry
// the tool result's people-only `details`; a large one (raw pages) is not
// inlined in the step read, so it is fetched as that one event.

import { useUiTranslations } from "@/providers/locale-provider";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { WebSearchDrawer } from "@/components/chat/web-search-details";
import { api } from "@/lib/api/client";
import type { Event, ListResponse, TraceStepDetail } from "@/lib/api/types";
import {
  attemptStatus,
  formatSearchDuration,
  parseWebSearchDetails,
  summarizeWebSearch,
} from "@/lib/web-search";
import { cn } from "@/lib/utils";

function useCompletedEventData(sessionId: string, detail: TraceStepDetail): unknown {
  const ref = detail.events.find((event) => event.type === "tool.completed");
  const inline = ref?.data;
  const query = useQuery({
    queryKey: ["trace-web-search-event", sessionId, ref?.sequence],
    enabled: !!ref && inline === undefined,
    queryFn: async () => {
      const params = new URLSearchParams({
        types: "tool.completed",
        before_sequence: String(ref!.sequence + 1),
        limit: "1",
      });
      const response = await api.get<ListResponse<Event>>(
        `/v1/sessions/${encodeURIComponent(sessionId)}/events?${params}`,
      );
      return response.data.data.find((event) => event.sequence === ref!.sequence)?.data ?? null;
    },
  });
  return inline ?? query.data;
}

export function WebSearchTraceSection({
  sessionId,
  detail,
}: {
  sessionId: string;
  detail: TraceStepDetail;
}) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(false);
  const data = useCompletedEventData(sessionId, detail) as
    | { details?: unknown; duration_ms?: number | null }
    | null
    | undefined;
  const details = parseWebSearchDetails(data?.details);
  if (!details) return null;
  const summary = summarizeWebSearch(details, data?.duration_ms);
  return (
    <section className="border-b px-5 py-4" aria-label={ui("Web search")}>
      <div className="mb-2.5 flex items-center gap-2">
        <h3 className="text-[10px] font-medium tracking-[0.12em] text-muted-foreground uppercase">
          {ui("Web search")}
        </h3>
        <span className="font-mono text-[11px] text-muted-foreground">
          {summary.providerLabel ?? ui("no answer")} · {summary.resultCount} {ui("results")}
        </span>
        <Button
          variant="link"
          size="sm"
          className="ml-auto h-auto p-0 text-xs"
          onClick={() => setOpen(true)}
        >
          {ui("Details and raw")}
        </Button>
      </div>
      <ul className="ml-1 border-l pl-3 text-xs">
        {details.attempts.map((attempt, index) => (
          <li
            key={`${attempt.provider}-${index}`}
            className={cn(
              "flex gap-2 py-0.5",
              !attempt.ok && attempt.duration_ms > 0 && "text-destructive",
            )}
          >
            <span className="font-mono text-muted-foreground">{index + 1}</span>
            <span className="min-w-0 flex-1 truncate">{attempt.label}</span>
            <span className="shrink-0">
              {attemptStatus(attempt)} · {formatSearchDuration(attempt.duration_ms)}
            </span>
          </li>
        ))}
      </ul>
      <WebSearchDrawer
        open={open}
        onOpenChange={setOpen}
        details={details}
        durationMs={data?.duration_ms}
        event={data}
      />
    </section>
  );
}
