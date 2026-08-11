"use client";

// Timeline is the curated execution history of a session recording. It streams
// live and replays from durable history, but never duplicates the conversation
// transcript or exposes mutation controls.

import { useSessionTasks } from "@/hooks/use-session-tasks";
import { RunTimeline } from "@/components/session/run-timeline";
import { useSessionContext } from "../session-context";

export default function TimelinePage() {
  const { sessionId, events, eventsLoading } = useSessionContext();
  const { data: tasks, isLoading: tasksLoading } = useSessionTasks(sessionId);

  return (
    <section
      aria-label="Execution timeline"
      className="min-h-0 flex-1 overflow-y-auto px-4 sm:px-6"
    >
      <div className="w-full">
        <RunTimeline
          events={events ?? []}
          tasks={tasks ?? []}
          loading={eventsLoading || tasksLoading}
        />
      </div>
    </section>
  );
}
