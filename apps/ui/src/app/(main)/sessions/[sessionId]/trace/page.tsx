"use client";

// Trace: one debugging view of a session's turns, model calls, tool calls and
// sub-agents, read from the server's trace index rather than the event log.
//
// When other agents took part in the run, "Whole run" adds a timeline with one
// lane per agent above this session's turns
// (knowledge/runtime-resources/agents-using-agents.md, Whole run).

import { Suspense, useMemo, useState } from "react";
import { TraceView, latestTraceSequence } from "@/components/session/trace/trace-view";
import { RunScopeSwitch, RunTimeline } from "@/components/session/run-view";
import { useSessionRun } from "@/hooks/use-sessions";
import { useSessionContext } from "../session-context";

export default function TracePage() {
  const { sessionId, events, isActive } = useSessionContext();
  const liveSequence = useMemo(() => latestTraceSequence(events), [events]);
  const { data: run } = useSessionRun(sessionId, isActive);
  const [scope, setScope] = useState<"session" | "run">("session");
  const multi = (run?.sessions.length ?? 0) > 1;
  return (
    <Suspense>
      {multi && run ? (
        <div className="flex max-h-[45vh] shrink-0 flex-col gap-4 overflow-y-auto px-6 pt-6 pb-2">
          <RunScopeSwitch scope={scope} onChange={setScope} agents={run.sessions.length} />
          {scope === "run" ? <RunTimeline run={run} sessionId={sessionId} /> : null}
        </div>
      ) : null}
      <TraceView sessionId={sessionId} liveSequence={liveSequence} />
    </Suspense>
  );
}
