// Session task hooks — snapshot via REST, live updates via the session SSE
// stream (snapshot-then-delta).
//
// There is no shared session event bus in the UI (the chat hook's stream is
// chat-local), so this hook opens its own EventSource on the session SSE
// endpoint. `task.created`/`task.updated` carry full task snapshots, so the
// list cache is patched in place by `task.id`; `task.message.*` events only
// invalidate the per-task detail key.
"use client";

import { useCallback, useEffect, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  cancelSessionTask,
  getSessionTask,
  listSessionTasks,
  postSessionTaskMessage,
} from "@/lib/api/session-tasks";
import { getSseUrl } from "@/lib/api/events";
import { createEventStream, type EventStreamLike } from "@/lib/event-stream";
import { createReconnectTracker } from "@/lib/sse-reconnect";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type { PostTaskMessageRequest, SessionTask, TaskMessage } from "@/lib/api/types";

/** Replace the matching task by ID, or append when new. */
export function upsertTask(tasks: SessionTask[] | undefined, task: SessionTask): SessionTask[] {
  if (!tasks) return [task];
  const index = tasks.findIndex((t) => t.id === task.id);
  if (index === -1) return [...tasks, task];
  const next = [...tasks];
  next[index] = task;
  return next;
}

/** List a session's tasks with live SSE-driven cache updates. */
export function useSessionTasks(sessionId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const queryClient = useQueryClient();
  const enabled = !!org && !!sessionId;

  const query = useQuery({
    queryKey: queryKeys.sessionTasks.list(sessionId!),
    queryFn: () => listSessionTasks(sessionId!),
    enabled,
  });

  const eventSourceRef = useRef<EventStreamLike | null>(null);
  const reconnectRef = useRef(createReconnectTracker());

  const cleanup = useCallback(() => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (!enabled || !sessionId) {
      cleanup();
      return;
    }

    reconnectRef.current = createReconnectTracker();
    let cancelled = false;

    const connectSSE = () => {
      cleanup();

      // No since_id and no event-type filter: the server rejects unknown
      // event types in filters, and task.* events are full snapshots patched
      // over a fresh list fetched on every connect.
      const eventSource = createEventStream(getSseUrl(sessionId), { withCredentials: true });
      eventSourceRef.current = eventSource;

      eventSource.addEventListener("connected", () => {
        reconnectRef.current.reset();
        // Close the fetch-then-subscribe gap: refetch the snapshot on every
        // connect (initial + reconnects) so missed events are recovered.
        queryClient.invalidateQueries({ queryKey: queryKeys.sessionTasks.list(sessionId) });
      });

      eventSource.addEventListener("disconnecting", (messageEvent) => {
        try {
          const data = JSON.parse(messageEvent.data) as { retry_ms?: number };
          const retryMs = reconnectRef.current.onGraceful(data.retry_ms ?? 1000);
          cleanup();
          setTimeout(() => {
            if (!cancelled) connectSSE();
          }, retryMs);
        } catch {
          cleanup();
          if (!cancelled) connectSSE();
        }
      });

      // task.created / task.updated carry full task snapshots.
      const onTaskSnapshot = (messageEvent: MessageEvent) => {
        try {
          const event = JSON.parse(messageEvent.data) as { data?: { task?: SessionTask } };
          const task = event.data?.task;
          if (!task) return;
          queryClient.setQueryData<SessionTask[]>(queryKeys.sessionTasks.list(sessionId), (tasks) =>
            upsertTask(tasks, task),
          );
        } catch (e) {
          console.error("Failed to parse session task event:", e);
        }
      };
      eventSource.addEventListener("task.created", onTaskSnapshot);
      eventSource.addEventListener("task.updated", onTaskSnapshot);

      // task.message.* only invalidate the per-task detail thread.
      const onTaskMessage = (messageEvent: MessageEvent) => {
        try {
          const event = JSON.parse(messageEvent.data) as { data?: { task_id?: string } };
          const taskId = event.data?.task_id;
          if (!taskId) return;
          queryClient.invalidateQueries({
            queryKey: queryKeys.sessionTasks.detail(sessionId, taskId),
          });
        } catch (e) {
          console.error("Failed to parse session task message event:", e);
        }
      };
      eventSource.addEventListener("task.message.sent", onTaskMessage);
      eventSource.addEventListener("task.message.received", onTaskMessage);

      eventSource.onerror = () => {
        cleanup();
        const delayMs = reconnectRef.current.onError();
        if (delayMs === null) return;
        setTimeout(() => {
          if (!cancelled) connectSSE();
        }, delayMs);
      };
    };

    connectSSE();

    return () => {
      cancelled = true;
      cleanup();
    };
  }, [enabled, sessionId, cleanup, queryClient]);

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/** Fetch one task's snapshot plus its message thread.
 *
 * Live updates arrive through the list hook's SSE stream, which invalidates this
 * detail key on `task.message.*` events — so mount `useSessionTasks` on the same
 * view (the Tasks tab does) for the thread to stay current. */
export function useSessionTask(sessionId: string | undefined, taskId: string | undefined) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: queryKeys.sessionTasks.detail(sessionId!, taskId!),
    queryFn: () => getSessionTask(sessionId!, taskId!),
    enabled: !!org && !!sessionId && !!taskId,
  });
}

/** Request cooperative cancellation of a task; patches the returned snapshot into the list cache. */
export function useCancelSessionTask(sessionId: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (taskId: string) => cancelSessionTask(sessionId!, taskId),
    onSuccess: (task: SessionTask) => {
      if (!sessionId) return;
      queryClient.setQueryData<SessionTask[]>(queryKeys.sessionTasks.list(sessionId), (tasks) =>
        upsertTask(tasks, task),
      );
    },
  });
}

/** Send an inbound message (steering or input answer) to a task. */
export function useSendTaskMessage(sessionId: string | undefined) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      taskId,
      request,
    }: {
      taskId: string;
      request: PostTaskMessageRequest;
    }): Promise<TaskMessage> => postSessionTaskMessage(sessionId!, taskId, request),
    onSuccess: (_message, { taskId }) => {
      if (!sessionId) return;
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionTasks.detail(sessionId, taskId),
      });
      // Answering an input request returns the task to running; refetch the
      // list in case the SSE patch lags.
      queryClient.invalidateQueries({ queryKey: queryKeys.sessionTasks.list(sessionId) });
    },
  });
}
