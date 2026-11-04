// Session task hooks — snapshot via REST, live updates via the session SSE
// stream (snapshot-then-delta).
//
// Snapshot consumers share the chat transport when one is mounted;
// standalone task views retain their own session SSE subscription. `task.created`/`task.updated` carry full task snapshots, so the
// list cache is patched in place by `task.id`; `task.message.*` events only
// invalidate the per-task detail key.
"use client";

import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import {
  cancelSessionTask,
  getSessionTask,
  listSessionTasks,
  postSessionTaskMessage,
} from "@/lib/api/session-tasks";
import { getSseUrl } from "@/lib/api/events";
import { createEventStream, eventStreamScope, type EventStreamLike } from "@/lib/event-stream";
import { createReconnectTracker } from "@/lib/sse-reconnect";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type { PostTaskMessageRequest, SessionTask, TaskMessage } from "@/lib/api/types";

/** Replace the matching task by ID, or append when new. */
export function upsertTask(tasks: SessionTask[] | undefined, task: SessionTask): SessionTask[] {
  if (!tasks) return [task];
  const index = tasks.findIndex((t) => t.id === task.id);
  if (index === -1) return [...tasks, task];
  if (Date.parse(tasks[index].updated_at) > Date.parse(task.updated_at)) return tasks;
  const next = [...tasks];
  next[index] = task;
  return next;
}

// One SSE stream per session, shared by every `useSessionTasks` instance.
//
// Task chips in the header and the Work tab both subscribe to the same
// session, and the server caps streams per session (SSE_PER_SESSION_MAX). With
// a stream per hook instance, a couple of tabs on one session exhausted the
// cap and the client looped on 429 (Sentry EVERRUNS-1M). Ref-counting keeps
// the last subscriber's stream open and closes it when nobody listens.
type SessionTaskStream = { refs: number; dispose: () => void };
const sessionTaskStreams = new Map<string, SessionTaskStream>();

/** Open (or join) the shared task stream for a session. Returns a release
 *  function; the stream closes when the last subscriber releases it. */
export function acquireSessionTaskStream(
  sessionId: string,
  queryClient: QueryClient,
  org = "",
): () => void {
  const scope = eventStreamScope(queryClient, org);
  const key = `${scope}:${sessionId}`;
  let entry = sessionTaskStreams.get(key);
  if (!entry) {
    entry = { refs: 0, dispose: subscribeSessionTasks(sessionId, queryClient, scope) };
    sessionTaskStreams.set(key, entry);
  }
  const stream = entry;
  stream.refs += 1;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    stream.refs -= 1;
    if (stream.refs > 0) return;
    stream.dispose();
    if (sessionTaskStreams.get(key) === stream) sessionTaskStreams.delete(key);
  };
}

/** Subscribe to one session's SSE stream and patch task snapshots into the
 *  session task cache. Returns a disposer that stops reconnects and closes
 *  the stream. */
function subscribeSessionTasks(
  sessionId: string,
  queryClient: QueryClient,
  scope: string,
): () => void {
  const tracker = createReconnectTracker();
  let stream: EventStreamLike | null = null;
  let cancelled = false;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;

  const closeStream = () => {
    clearTimeout(retryTimer);
    if (stream) {
      stream.close();
      stream = null;
    }
  };

  const connect = () => {
    closeStream();

    // No since_id and no event-type filter: the server rejects unknown
    // event types in filters, and task.* events are full snapshots patched
    // over a fresh list fetched on every connect.
    const source = createEventStream(getSseUrl(sessionId), {
      withCredentials: true,
      shareScope: scope,
      snapshotOnly: true,
    });
    stream = source;
    const isCurrent = () => !cancelled && stream === source;

    source.addEventListener("connected", () => {
      if (!isCurrent()) return;
      tracker.reset();
      // Close the fetch-then-subscribe gap: refetch the snapshot on every
      // connect (initial + reconnects) so missed events are recovered.
      queryClient.invalidateQueries({ queryKey: queryKeys.sessionTasks.list(sessionId) });
    });

    source.addEventListener("disconnecting", (messageEvent) => {
      if (!isCurrent()) return;
      try {
        const data = JSON.parse(messageEvent.data) as { retry_ms?: number };
        const retryMs = tracker.onGraceful(data.retry_ms ?? 1000);
        closeStream();
        retryTimer = setTimeout(() => {
          if (!cancelled) connect();
        }, retryMs);
      } catch {
        closeStream();
        if (!cancelled) connect();
      }
    });

    // task.created / task.updated carry full task snapshots.
    const onTaskSnapshot = (messageEvent: MessageEvent) => {
      if (!isCurrent()) return;
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
    source.addEventListener("task.created", onTaskSnapshot);
    source.addEventListener("task.updated", onTaskSnapshot);

    // task.message.* only invalidate the per-task detail thread.
    const onTaskMessage = (messageEvent: MessageEvent) => {
      if (!isCurrent()) return;
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
    source.addEventListener("task.message.sent", onTaskMessage);
    source.addEventListener("task.message.received", onTaskMessage);

    source.onerror = () => {
      if (!isCurrent()) return;
      closeStream();
      const delayMs = tracker.onError();
      if (delayMs === null) return;
      retryTimer = setTimeout(() => {
        if (!cancelled) connect();
      }, delayMs);
    };
  };

  const wake = () => {
    if (!cancelled && navigator.onLine && !stream) {
      tracker.forceReset();
      connect();
    }
  };
  const visible = () => {
    if (document.visibilityState === "visible") wake();
  };
  window.addEventListener("online", wake);
  window.addEventListener("focus", wake);
  document.addEventListener("visibilitychange", visible);
  connect();

  return () => {
    window.removeEventListener("online", wake);
    window.removeEventListener("focus", wake);
    document.removeEventListener("visibilitychange", visible);
    cancelled = true;
    closeStream();
  };
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

  useEffect(() => {
    if (!enabled || !sessionId) return;
    return acquireSessionTaskStream(sessionId, queryClient, org);
  }, [enabled, sessionId, queryClient, org]);

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
