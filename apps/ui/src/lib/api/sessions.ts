// Session and Message API functions (M2)
// Messages are PRIMARY data, Events are SSE notifications

import { api } from "./client";
import type {
  Session,
  Message,
  CreateSessionRequest,
  UpdateSessionRequest,
  CreateMessageRequest,
  ListResponse,
} from "./types";

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

// ============================================
// Messages (PRIMARY data)
// ============================================

export async function createMessage(
  agentId: string,
  sessionId: string,
  request: CreateMessageRequest
): Promise<Message> {
  const response = await api.post<Message>(
    `/v1/agents/${agentId}/sessions/${sessionId}/messages`,
    request
  );
  return response.data;
}

export async function listMessages(
  agentId: string,
  sessionId: string
): Promise<Message[]> {
  const response = await api.get<ListResponse<Message>>(
    `/v1/agents/${agentId}/sessions/${sessionId}/messages`
  );
  return response.data.data;
}

// Send a user message to a session (triggers workflow)
export async function sendUserMessage(
  agentId: string,
  sessionId: string,
  content: string,
  controls?: { reasoning_effort?: string }
): Promise<Message> {
  const request: CreateMessageRequest = {
    message: {
      role: "user",
      content: [{ type: "text", text: content }],
    },
  };

  // Add controls if reasoning_effort is specified
  if (controls?.reasoning_effort) {
    request.controls = {
      reasoning: {
        effort: controls.reasoning_effort,
      },
    };
  }

  return createMessage(agentId, sessionId, request);
}

// ============================================
// Events (SSE notifications)
// ============================================

// SSE event stream URL builder
export function getEventStreamUrl(agentId: string, sessionId: string): string {
  const baseUrl = api.defaults.baseURL || "";
  return `${baseUrl}/v1/agents/${agentId}/sessions/${sessionId}/events`;
}
