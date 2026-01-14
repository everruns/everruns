// Session API functions

import { api } from "./client";
import type {
  Session,
  CreateSessionRequest,
  UpdateSessionRequest,
  PaginatedResponse,
  PaginationParams,
} from "./types";

// Re-export message and event functions for convenience
export { createMessage, listMessages, sendUserMessage } from "./messages";
export { listEvents } from "./events";

// ============================================
// Session CRUD
// ============================================

export async function createSession(
  agentId: string,
  request: CreateSessionRequest = {}
): Promise<Session> {
  const response = await api.post<Session>(
    `/v1/agents/${agentId}/sessions`,
    request
  );
  return response.data;
}

export async function listSessions(
  agentId: string,
  params?: PaginationParams
): Promise<PaginatedResponse<Session>> {
  const searchParams = new URLSearchParams();
  if (params?.offset !== undefined) {
    searchParams.set("offset", String(params.offset));
  }
  if (params?.limit !== undefined) {
    searchParams.set("limit", String(params.limit));
  }
  const query = searchParams.toString();
  const url = `/v1/agents/${agentId}/sessions${query ? `?${query}` : ""}`;
  const response = await api.get<PaginatedResponse<Session>>(url);
  return response.data;
}

export async function getSession(
  agentId: string,
  sessionId: string
): Promise<Session> {
  const response = await api.get<Session>(
    `/v1/agents/${agentId}/sessions/${sessionId}`
  );
  return response.data;
}

export async function updateSession(
  agentId: string,
  sessionId: string,
  request: UpdateSessionRequest
): Promise<Session> {
  const response = await api.patch<Session>(
    `/v1/agents/${agentId}/sessions/${sessionId}`,
    request
  );
  return response.data;
}

export async function deleteSession(
  agentId: string,
  sessionId: string
): Promise<void> {
  await api.delete(`/v1/agents/${agentId}/sessions/${sessionId}`);
}
