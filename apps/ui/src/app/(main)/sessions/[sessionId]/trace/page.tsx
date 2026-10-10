"use client";

// Trace: one debugging view of a session's turns, model calls, tool calls and
// sub-agents, read from the server's trace index rather than the event log.

import { Suspense, useMemo } from "react";
import { TraceView, latestTraceSequence } from "@/components/session/trace/trace-view";
import { useSessionContext } from "../session-context";

export default function TracePage() {
  const { sessionId, events } = useSessionContext();
  const liveSequence = useMemo(() => latestTraceSequence(events), [events]);
  return (
    <Suspense>
      <TraceView sessionId={sessionId} liveSequence={liveSequence} />
    </Suspense>
  );
}
