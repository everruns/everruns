"use client";

// Timeline — the default view of a session recording (EVE-854): the structural
// run on the left, the transcript on the right. Both are read-only and both
// stream: a live session appends to each as events arrive, a finished one
// replays entirely from history.

import { useSessionTasks } from "@/hooks/use-session-tasks";
import { RunTimeline } from "@/components/session/run-timeline";
import { SessionTranscript } from "@/components/session/session-transcript";
import { useSessionContext } from "../session-context";

export default function TimelinePage() {
  const { sessionId, events, eventsLoading } = useSessionContext();
  const { data: tasks, isLoading: tasksLoading } = useSessionTasks(sessionId);

  return (
    <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
      <aside className="flex min-h-0 shrink-0 flex-col overflow-y-auto border-b border-border/70 px-4 lg:w-96 lg:border-b-0 lg:border-r">
        <h2 className="pt-4 text-sm font-medium text-muted-foreground">Run</h2>
        <RunTimeline
          events={events ?? []}
          tasks={tasks ?? []}
          loading={eventsLoading || tasksLoading}
        />
      </aside>

      <div className="flex min-h-0 flex-1 flex-col">
        <SessionTranscript />
      </div>
    </div>
  );
}
