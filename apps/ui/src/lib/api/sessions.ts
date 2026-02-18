// Session API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import type {
  Session,
  CreateSessionRequest,
  UpdateSessionRequest,
  PaginatedResponse,
  PaginationParams,
  SessionSchedulesResponse,
  SessionScheduleSummary,
} from "./types";

// Re-export message and event functions for convenience
export { createMessage, listMessages, sendUserMessage } from "./messages";
export { listEvents } from "./events";

// ============================================
// Session CRUD
// ============================================

/**
 * Create a new session for an agent.
 * Sessions are direct children of organizations, with agent_id specifying which agent works in the session.
 */
export async function createSession(request: CreateSessionRequest): Promise<Session> {
  const response = await api.post<Session>("/v1/sessions", request);
  return response.data;
}

/**
 * List sessions for an organization.
 * @param agentId - Optional filter by agent ID
 */
export async function listSessions(
  params?: PaginationParams & { agentId?: string },
): Promise<PaginatedResponse<Session>> {
  const searchParams = new URLSearchParams();
  if (params?.agentId) {
    searchParams.set("agent_id", params.agentId);
  }
  if (params?.offset !== undefined) {
    searchParams.set("offset", String(params.offset));
  }
  if (params?.limit !== undefined) {
    searchParams.set("limit", String(params.limit));
  }
  const query = searchParams.toString();
  const url = `/v1/sessions${query ? `?${query}` : ""}`;
  const response = await api.get<PaginatedResponse<Session>>(url);
  return response.data;
}

export async function getSession(sessionId: string): Promise<Session> {
  const response = await api.get<Session>(`/v1/sessions/${sessionId}`);
  return response.data;
}

export async function updateSession(
  sessionId: string,
  request: UpdateSessionRequest,
): Promise<Session> {
  const response = await api.patch<Session>(`/v1/sessions/${sessionId}`, request);
  return response.data;
}

export async function deleteSession(sessionId: string): Promise<void> {
  await api.delete(`/v1/sessions/${sessionId}`);
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
export async function cancelTurn(sessionId: string): Promise<void> {
  await api.post(`/v1/sessions/${sessionId}/cancel`);
}

// ============================================
// Session Pinning
// ============================================

/** Pin a session for the current user */
export async function pinSession(sessionId: string): Promise<void> {
  await api.put(`/v1/sessions/${sessionId}/pin`);
}

/** Unpin a session for the current user */
export async function unpinSession(sessionId: string): Promise<void> {
  await api.delete(`/v1/sessions/${sessionId}/pin`);
}

// ============================================
// Session Schedules
// ============================================

/** List all schedules for a session */
export async function listSessionSchedules(sessionId: string): Promise<SessionSchedulesResponse> {
  const response = await api.get<SessionSchedulesResponse>(`/v1/sessions/${sessionId}/schedules`);
  return response.data;
}

/** Get schedule summary (pending count) for a session */
export async function getSessionScheduleSummary(
  sessionId: string,
): Promise<SessionScheduleSummary> {
  const response = await api.get<SessionScheduleSummary>(
    `/v1/sessions/${sessionId}/schedules/summary`,
  );
  return response.data;
}
