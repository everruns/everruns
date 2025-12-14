import { apiClient } from "./client";
import type { Thread, Message, CreateMessageRequest } from "./types";

// Thread CRUD
export async function createThread(): Promise<Thread> {
  return apiClient<Thread>("/v1/threads", {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function getThread(threadId: string): Promise<Thread> {
  return apiClient<Thread>(`/v1/threads/${threadId}`);
}

// Messages
export async function getMessages(threadId: string): Promise<Message[]> {
  return apiClient<Message[]>(`/v1/threads/${threadId}/messages`);
}

export async function createMessage(
  threadId: string,
  data: CreateMessageRequest
): Promise<Message> {
  return apiClient<Message>(`/v1/threads/${threadId}/messages`, {
    method: "POST",
    body: JSON.stringify(data),
  });
}
