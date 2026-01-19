// Session API functions
// All routes are org-scoped: /v1/orgs/{org}/agents/{agentId}/sessions/...

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
  org: string,
  agentId: string,
  request: CreateSessionRequest = {}
): Promise<Session> {
  const response = await api.post<Session>(
    `/v1/orgs/${org}/agents/${agentId}/sessions`,
    request
  );
  return response.data;
}

export async function listSessions(
  org: string,
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
  const url = `/v1/orgs/${org}/agents/${agentId}/sessions${query ? `?${query}` : ""}`;
  const response = await api.get<PaginatedResponse<Session>>(url);
  return response.data;
}

export async function getSession(
  org: string,
  agentId: string,
  sessionId: string
): Promise<Session> {
  const response = await api.get<Session>(
    `/v1/orgs/${org}/agents/${agentId}/sessions/${sessionId}`
  );
  return response.data;
}

export async function updateSession(
  org: string,
  agentId: string,
  sessionId: string,
  request: UpdateSessionRequest
): Promise<Session> {
  const response = await api.patch<Session>(
    `/v1/orgs/${org}/agents/${agentId}/sessions/${sessionId}`,
    request
  );
  return response.data;
}

export async function deleteSession(
  org: string,
  agentId: string,
  sessionId: string
): Promise<void> {
  await api.delete(`/v1/orgs/${org}/agents/${agentId}/sessions/${sessionId}`);
}

/**
 * Cancel the currently running turn in a session.
 *
 * This will:
 * 1. Cancel the underlying workflow execution
 * 2. Emit a turn.cancelled event
 * 3. Insert an agent message indicating the turn was cancelled
 * 4. Set the session status back to idle
 *
 * @throws Error if no turn is currently running (409 Conflict)
 */
export async function cancelTurn(
  org: string,
  agentId: string,
  sessionId: string
): Promise<void> {
  await api.post(`/v1/orgs/${org}/agents/${agentId}/sessions/${sessionId}/cancel`);
}
