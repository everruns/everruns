import type { Session, SessionTask } from "@/lib/api/types";

export const THREAD_GROUPS = ["Needs you", "Working", "Open", "Resolved"] as const;
export type ThreadGroup = (typeof THREAD_GROUPS)[number];

export function conversationGroup(session: Session): ThreadGroup {
  if (session.status === "waiting_for_tool_results") return "Needs you";
  if (session.status === "active" || session.status === "started") return "Working";
  // Turn completion is idle, not resolution. Only an explicit archive resolves a conversation.
  return session.archived_at ? "Resolved" : "Open";
}

export function workGroup(task: SessionTask): ThreadGroup {
  if (task.state === "awaiting_input" || task.state === "failed") return "Needs you";
  if (task.state === "running" || task.state === "queued") return "Working";
  return "Resolved";
}
