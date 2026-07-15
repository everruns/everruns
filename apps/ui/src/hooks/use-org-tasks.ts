// Cross-session org task hook (EVE-756 Work view).
//
// Snapshot via the org-wide `GET /v1/tasks`, then live snapshot-then-delta by
// task_id — identical to `useSessionTasks`, but fanned out across sessions.
//
// There is no org-wide SSE endpoint; the event stream is per-session
// (`/v1/sessions/{id}/sse`). To avoid turning an org-wide task snapshot into
// unbounded persistent streams, this hook subscribes only to a capped newest-task
// session window and reconciles `task.created` / `task.updated` (full snapshots)
// into the single org task cache by task_id.
"use client";

import { useEffect, useMemo } from "react";
import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { listOrgTasks } from "@/lib/api/session-tasks";
import { upsertTask } from "@/hooks/use-session-tasks";
import { getSseUrl } from "@/lib/api/events";
import { createEventStream, type EventStreamLike } from "@/lib/event-stream";
import { createReconnectTracker } from "@/lib/sse-reconnect";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type { SessionTask } from "@/lib/api/types";

export const ORG_TASKS_LIVE_SESSION_LIMIT = 20;
export const ORG_TASKS_LIST_LIMIT = 100;

const connectedInvalidationTimers = new WeakMap<QueryClient, ReturnType<typeof setTimeout>>();

function invalidateOrgTasksSoon(queryClient: QueryClient) {
  if (connectedInvalidationTimers.has(queryClient)) return;
  const timer = setTimeout(() => {
    connectedInvalidationTimers.delete(queryClient);
    queryClient.invalidateQueries({ queryKey: queryKeys.orgTasks.list() });
  }, 0);
  connectedInvalidationTimers.set(queryClient, timer);
}

export function liveOrgTaskSessionIds(tasks: SessionTask[]): string[] {
  const ids = new Set<string>();
  for (const task of tasks) {
    ids.add(task.session_id);
    if (ids.size >= ORG_TASKS_LIVE_SESSION_LIMIT) break;
  }
  return Array.from(ids).sort();
}

/** Subscribe to one session's SSE stream, reconciling task snapshots into the
 *  shared org task cache. Returns a disposer that stops reconnects and closes
 *  the stream. */
function subscribeSession(sessionId: string, queryClient: QueryClient): () => void {
  const tracker = createReconnectTracker();
  let stream: EventStreamLike | null = null;
  let cancelled = false;

  const closeStream = () => {
    if (stream) {
      stream.close();
      stream = null;
    }
  };

  const connect = () => {
    closeStream();
    // No since_id / event-type filter: task.* events are full snapshots patched
    // over a fresh list fetched on every connect.
    const source = createEventStream(getSseUrl(sessionId), { withCredentials: true });
    stream = source;

    source.addEventListener("connected", () => {
      tracker.reset();
      // Close the fetch-then-subscribe gap without refetching once per stream.
      invalidateOrgTasksSoon(queryClient);
    });

    source.addEventListener("disconnecting", (messageEvent) => {
      try {
        const data = JSON.parse(messageEvent.data) as { retry_ms?: number };
        const retryMs = tracker.onGraceful(data.retry_ms ?? 1000);
        closeStream();
        setTimeout(() => {
          if (!cancelled) connect();
        }, retryMs);
      } catch {
        closeStream();
        if (!cancelled) connect();
      }
    });

    const onTaskSnapshot = (messageEvent: MessageEvent) => {
      try {
        const event = JSON.parse(messageEvent.data) as { data?: { task?: SessionTask } };
        const task = event.data?.task;
        if (!task) return;
        queryClient.setQueryData<SessionTask[]>(queryKeys.orgTasks.list(), (tasks) =>
          upsertTask(tasks, task),
        );
      } catch (e) {
        console.error("Failed to parse org task event:", e);
      }
    };
    source.addEventListener("task.created", onTaskSnapshot);
    source.addEventListener("task.updated", onTaskSnapshot);

    // task.message.* only invalidate the per-task detail thread, keyed by the
    // owning session so the reused per-session detail cache stays fresh.
    const onTaskMessage = (messageEvent: MessageEvent) => {
      try {
        const event = JSON.parse(messageEvent.data) as { data?: { task_id?: string } };
        const taskId = event.data?.task_id;
        if (!taskId) return;
        queryClient.invalidateQueries({
          queryKey: queryKeys.sessionTasks.detail(sessionId, taskId),
        });
      } catch (e) {
        console.error("Failed to parse org task message event:", e);
      }
    };
    source.addEventListener("task.message.sent", onTaskMessage);
    source.addEventListener("task.message.received", onTaskMessage);

    source.onerror = () => {
      closeStream();
      const delayMs = tracker.onError();
      if (delayMs === null) return;
      setTimeout(() => {
        if (!cancelled) connect();
      }, delayMs);
    };
  };

  connect();

  return () => {
    cancelled = true;
    closeStream();
  };
}

/** List every task in the caller's org with live SSE-driven cache updates. */
export function useOrgTasks() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const queryClient = useQueryClient();
  const enabled = !!org;

  const query = useQuery({
    queryKey: queryKeys.orgTasks.list(),
    queryFn: () => listOrgTasks({ limit: ORG_TASKS_LIST_LIMIT }),
    enabled,
  });

  // Capped owning sessions in the newest-first snapshot, as a stable key so the
  // effect only re-subscribes when the live session window actually changes.
  const sessionIdsKey = useMemo(
    () => liveOrgTaskSessionIds(query.data ?? []).join(","),
    [query.data],
  );

  useEffect(() => {
    if (!enabled || !sessionIdsKey) return;
    const sessionIds = sessionIdsKey.split(",");
    const disposers = sessionIds.map((id) => subscribeSession(id, queryClient));
    return () => {
      for (const dispose of disposers) dispose();
    };
  }, [enabled, sessionIdsKey, queryClient]);

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}
