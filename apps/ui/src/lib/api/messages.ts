// Message API functions

import { api } from "./client";
import type {
  Message,
  CreateMessageRequest,
  ListResponse,
  Controls,
} from "./types";

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
  controls?: Controls
): Promise<Message> {
  return createMessage(agentId, sessionId, {
    message: {
      role: "user",
      content: [{ type: "text", text: content }],
    },
    controls,
  });
}
