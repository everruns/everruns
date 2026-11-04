"use client";

// Agents: the whole run this session belongs to, for debugging. Every
// hand-off with status, time and cost; a row opens that agent's session.
// Read-only: requests to the person are answered in the conversation the
// person is in, never here (knowledge/runtime-resources/agents-using-agents.md).

import { useUiTranslations } from "@/providers/locale-provider";
import { useSessionRun } from "@/hooks/use-sessions";
import { Skeleton } from "@/components/ui/skeleton";
import { RunHandoffs, RunSummaryCards } from "@/components/session/run-view";
import { useSessionContext } from "../session-context";

export default function AgentsPage() {
  const ui = useUiTranslations();
  const { sessionId, isActive } = useSessionContext();
  const { data: run, isLoading } = useSessionRun(sessionId, isActive);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 p-6">
        {isLoading ? (
          <Skeleton className="h-72 w-full" />
        ) : run ? (
          <>
            <RunSummaryCards run={run} />
            <RunHandoffs run={run} sessionId={sessionId} />
          </>
        ) : (
          <p className="border border-border bg-card p-8 text-sm text-muted-foreground">
            {ui("The run is not available for this session.")}
          </p>
        )}
      </div>
    </div>
  );
}
