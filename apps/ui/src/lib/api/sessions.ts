// Session API functions

import { api } from "./client";
import type {
  Session,
  CreateSessionRequest,
  UpdateSessionRequest,
  ListResponse,
} from "./types";

// Re-export message functions for backward compatibility
export {
  createMessage,
  listMessages,
  sendUserMessage,
  listEvents,
  getEventStreamUrl,
} from "./messages";

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

export async function listSessions(agentId: string): Promise<Session[]> {
  const response = await api.get<ListResponse<Session>>(
    `/v1/agents/${agentId}/sessions`
  );
  return response.data.data;
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
