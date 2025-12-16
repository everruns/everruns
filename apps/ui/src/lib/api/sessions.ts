// Session API functions (M2)

import { api } from "./client";
import type {
  Session,
  Event,
  CreateSessionRequest,
  UpdateSessionRequest,
  CreateEventRequest,
  ListResponse,
} from "./types";

export async function createSession(
  harnessId: string,
  request: CreateSessionRequest = {}
): Promise<Session> {
  const response = await api.post<Session>(
    `/v1/harnesses/${harnessId}/sessions`,
    request
  );
  return response.data;
}

export async function listSessions(harnessId: string): Promise<Session[]> {
  const response = await api.get<ListResponse<Session>>(
    `/v1/harnesses/${harnessId}/sessions`
  );
  return response.data.data;
}

export async function getSession(
  harnessId: string,
  sessionId: string
): Promise<Session> {
  const response = await api.get<Session>(
    `/v1/harnesses/${harnessId}/sessions/${sessionId}`
  );
  return response.data;
}

export async function updateSession(
  harnessId: string,
  sessionId: string,
  request: UpdateSessionRequest
): Promise<Session> {
  const response = await api.patch<Session>(
    `/v1/harnesses/${harnessId}/sessions/${sessionId}`,
    request
  );
  return response.data;
}

export async function deleteSession(
  harnessId: string,
  sessionId: string
): Promise<void> {
  await api.delete(`/v1/harnesses/${harnessId}/sessions/${sessionId}`);
}

// Event functions
export async function createEvent(
  harnessId: string,
  sessionId: string,
  request: CreateEventRequest
): Promise<Event> {
  const response = await api.post<Event>(
    `/v1/harnesses/${harnessId}/sessions/${sessionId}/events`,
    request
  );
  return response.data;
}

export async function listMessages(
  harnessId: string,
  sessionId: string
): Promise<Event[]> {
  const response = await api.get<ListResponse<Event>>(
    `/v1/harnesses/${harnessId}/sessions/${sessionId}/messages`
  );
  return response.data.data;
}

// SSE event stream URL builder
export function getEventStreamUrl(harnessId: string, sessionId: string): string {
  const baseUrl = api.defaults.baseURL || "";
  return `${baseUrl}/v1/harnesses/${harnessId}/sessions/${sessionId}/events`;
}

// Send a user message to a session
export async function sendUserMessage(
  harnessId: string,
  sessionId: string,
  content: string
): Promise<Event> {
  return createEvent(harnessId, sessionId, {
    event_type: "message.user",
    data: {
      message: {
        role: "user",
        content: [{ type: "text", text: content }],
      },
    },
  });
}
