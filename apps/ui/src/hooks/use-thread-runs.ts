"use client";

// Runs a chat thread produced. The thread's own tasks arrive live over the
// session SSE stream (`useSessionTasks`); the delegation tree is fetched only to
// count subagents underneath a run, and only when a run actually has a child
// session to count into.

import { useMemo } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { listOrgTasks } from "@/lib/api/session-tasks";
import { queryKeys } from "@/lib/query-keys";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import type { ChatRun } from "@/components/chat/run-cards";
import type { SessionTask } from "@/lib/api/types";

/** Cap on a thread's delegation tree. Well past any real thread; a runaway tree
 *  is truncated rather than paged, and only affects the subagent count. */
const TREE_TASK_LIMIT = 500;

/** Runs started by this thread, oldest first. Pass `undefined` to stay inert —
 *  the hook then opens no SSE stream and issues no request. */
export function useThreadRuns(sessionId: string | undefined): ChatRun[] {
  const { data: tasks } = useSessionTasks(sessionId);

  const threadTasks = useMemo<SessionTask[]>(() => {
    if (!sessionId || !tasks) return [];
    return [...tasks]
      .filter((task) => task.session_id === sessionId)
      .sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at));
  }, [tasks, sessionId]);

  // Refetch the tree when the thread's own tasks change — a new run appearing or
  // a run finishing is exactly when its subagent count can have moved.
  const signature = threadTasks.map((task) => `${task.id}:${task.state}`).join(",");
  const childSessionIds = useMemo(
    () =>
      new Set(
        threadTasks.map((task) => task.links?.child_session_id).filter((id): id is string => !!id),
      ),
    [threadTasks],
  );

  const tree = useQuery({
    queryKey: queryKeys.orgTasks.tree(sessionId ?? "", signature),
    queryFn: () => listOrgTasks({ root_session_id: sessionId, limit: TREE_TASK_LIMIT }),
    // Gated on the thread's own tasks having arrived, which already implies an
    // org context; no separate org check is needed here.
    enabled: !!sessionId && childSessionIds.size > 0,
    placeholderData: keepPreviousData,
  });

  return useMemo(() => {
    const subagentCounts = new Map<string, number>();
    for (const task of tree.data ?? []) {
      if (!childSessionIds.has(task.session_id)) continue;
      subagentCounts.set(task.session_id, (subagentCounts.get(task.session_id) ?? 0) + 1);
    }

    return threadTasks.map((task) => ({
      task,
      subagentCount: task.links?.child_session_id
        ? (subagentCounts.get(task.links.child_session_id) ?? 0)
        : 0,
    }));
  }, [threadTasks, tree.data, childSessionIds]);
}
