// Session task registry API functions (knowledge/runtime-resources/session-tasks.md)

import { api } from "./client";
import type {
  PostTaskMessageRequest,
  SessionTask,
  SessionTaskDetail,
  SessionTaskState,
  TaskMessage,
} from "./types";

export interface ListSessionTasksOptions {
  state?: SessionTaskState;
  kind?: string;
}

/** List tasks owned by a session, optionally filtered by state/kind. */
export async function listSessionTasks(
  sessionId: string,
  options?: ListSessionTasksOptions,
): Promise<SessionTask[]> {
  const params = new URLSearchParams();
  if (options?.state) params.set("state", options.state);
  if (options?.kind) params.set("kind", options.kind);
  const queryString = params.toString();
  const response = await api.get<SessionTask[]>(
    `/v1/sessions/${sessionId}/tasks${queryString ? `?${queryString}` : ""}`,
  );
  return response.data;
}

export interface ListOrgTasksOptions {
  state?: SessionTaskState;
  kind?: string;
  /** Only tasks whose owning session's delegation-tree root is this session. */
  root_session_id?: string;
  /** RFC3339 timestamp; only tasks created at or after this time. */
  created_after?: string;
  /** Max tasks to return (server default 100, max 500, newest first). */
  limit?: number;
}

/** List background tasks across every session in the caller's org.
 *
 *  Backs the cross-session Work view (EVE-756). The server returns a flat,
 *  newest-first array; grouping into a delegation tree by `root_session_id` is a
 *  UI concern. */
export async function listOrgTasks(options?: ListOrgTasksOptions): Promise<SessionTask[]> {
  const params = new URLSearchParams();
  if (options?.state) params.set("state", options.state);
  if (options?.kind) params.set("kind", options.kind);
  if (options?.root_session_id) params.set("root_session_id", options.root_session_id);
  if (options?.created_after) params.set("created_after", options.created_after);
  if (options?.limit != null) params.set("limit", String(options.limit));
  const queryString = params.toString();
  const response = await api.get<SessionTask[]>(`/v1/tasks${queryString ? `?${queryString}` : ""}`);
  return response.data;
}

/** Get one task snapshot plus its recent message thread. */
export async function getSessionTask(
  sessionId: string,
  taskId: string,
): Promise<SessionTaskDetail> {
  const response = await api.get<SessionTaskDetail>(`/v1/sessions/${sessionId}/tasks/${taskId}`);
  return response.data;
}

/** Post an inbound message (steering or input answer) to a task.
 *  Not supported for subagent tasks; the server returns 400 for that kind. */
export async function postSessionTaskMessage(
  sessionId: string,
  taskId: string,
  request: PostTaskMessageRequest,
): Promise<TaskMessage> {
  const response = await api.post<TaskMessage>(
    `/v1/sessions/${sessionId}/tasks/${taskId}/messages`,
    request,
  );
  return response.data;
}

/** Record cooperative cancel intent. Returns the task snapshot. */
export async function cancelSessionTask(sessionId: string, taskId: string): Promise<SessionTask> {
  const response = await api.post<SessionTask>(`/v1/sessions/${sessionId}/tasks/${taskId}/cancel`);
  return response.data;
}
