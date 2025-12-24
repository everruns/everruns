// Message API functions
// Messages are PRIMARY data, Events are SSE notifications

import { api } from "./client";
import type {
  Message,
  CreateMessageRequest,
  ListResponse,
} from "./types";

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
  content: string
): Promise<Message> {
  return createMessage(agentId, sessionId, {
    message: {
      role: "user",
      content: [{ type: "text", text: content }],
    },
  });
}

// ============================================
// Events (SSE notifications)
// ============================================

// SSE event stream URL builder
export function getEventStreamUrl(agentId: string, sessionId: string): string {
  const baseUrl = api.defaults.baseURL || "";
  return `${baseUrl}/v1/agents/${agentId}/sessions/${sessionId}/events`;
}
