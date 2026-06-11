// Session task registry API functions (specs/session-tasks.md)

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

/** Get one task snapshot plus its recent message thread. */
export async function getSessionTask(
  sessionId: string,
  taskId: string,
): Promise<SessionTaskDetail> {
  const response = await api.get<SessionTaskDetail>(`/v1/sessions/${sessionId}/tasks/${taskId}`);
  return response.data;
}

/** Post an inbound message (steering or input answer) to a task. */
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
