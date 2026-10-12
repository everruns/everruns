import type { Session, SessionTask } from "@/lib/api/types";

export const THREAD_GROUPS = [
  "Needs you",
  "Working",
  "Ready for review",
  "Open",
  "Resolved",
] as const;
export type ThreadGroup = (typeof THREAD_GROUPS)[number];

/** Task kind for one unit of work a coordinator handed to a thread. */
export const ASSIGNMENT_TASK_KIND = "assignment";
/** `state_detail` the coordinator writes when it resolves a thread. */
export const THREAD_RESOLVED_DETAIL = "resolved";

export function conversationGroup(session: Session): ThreadGroup {
  // An explicit archive resolves a conversation whatever its last status: an
  // archived session can be left `active` or `started` and must not read as work.
  if (session.archived_at) return "Resolved";
  if (session.status === "waiting_for_tool_results") return "Needs you";
  // `started` means no turn has run yet, so it is open, not working.
  if (session.status === "active") return "Working";
  // Turn completion is idle, not resolution.
  return "Open";
}

export function workGroup(task: SessionTask): ThreadGroup {
  if (task.state === "awaiting_input" || task.state === "failed") return "Needs you";
  if (task.state === "running" || task.state === "queued") return "Working";
  return "Resolved";
}

/** A thread's group comes from its latest assignment: finished work waits for
 *  review until the coordinator (or the person, through it) resolves the thread. */
export function assignmentGroup(task: SessionTask): ThreadGroup {
  if (task.state_detail === THREAD_RESOLVED_DETAIL) return "Resolved";
  if (task.state === "succeeded") return "Ready for review";
  if (task.state === "canceled") return "Open";
  return workGroup(task);
}

export function isAssignment(task: SessionTask): boolean {
  return task.kind === ASSIGNMENT_TASK_KIND && !!task.links?.child_session_id;
}

export type CoordinatorThread = {
  threadId: string;
  /** The thread's first assignment; its title names the thread. */
  first: SessionTask;
  /** The thread's latest assignment; it decides the group and the preview. */
  latest: SessionTask;
  assignments: number;
};

/** One entry per thread: a thread can carry many assignments over its life. */
export function coordinatorThreads(tasks: SessionTask[]): CoordinatorThread[] {
  const byThread = new Map<string, CoordinatorThread>();
  for (const task of tasks) {
    if (!isAssignment(task)) continue;
    const threadId = task.links!.child_session_id!;
    const entry = byThread.get(threadId);
    if (!entry) {
      byThread.set(threadId, { threadId, first: task, latest: task, assignments: 1 });
      continue;
    }
    entry.assignments += 1;
    if (task.created_at > entry.latest.created_at) entry.latest = task;
    if (task.created_at < entry.first.created_at) entry.first = task;
  }
  return [...byThread.values()];
}

/** A thread keeps the name it started with; later assignments show in its preview. */
export function threadRowTitle({ first }: CoordinatorThread): string {
  return first.display_name || "Thread";
}

/** The current assignment's title, when it differs from the thread's name. */
export function currentAssignmentLabel(thread: CoordinatorThread): string | null {
  const current = thread.latest.display_name;
  return current && current !== threadRowTitle(thread) ? current : null;
}

/** Short progress line from the worker's checklist, e.g. "2 of 5 steps done". */
export function checklistSummary(task: SessionTask): string | null {
  const steps = task.progress?.steps ?? [];
  if (!steps.length) return null;
  const done = steps.filter((step) => step.status === "done" || step.status === "skipped").length;
  const current = steps.find((step) => step.status === "in_progress");
  const count = `${done} of ${steps.length} steps done`;
  return current ? `${current.title} · ${count}` : count;
}
