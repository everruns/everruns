// Message API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import type {
  Message,
  CreateMessageRequest,
  ListResponse,
  Controls,
} from "./types";

export async function createMessage(
  sessionId: string,
  request: CreateMessageRequest
): Promise<Message> {
  const response = await api.post<Message>(
    `/v1/sessions/${sessionId}/messages`,
    request
  );
  return response.data;
}

export async function listMessages(sessionId: string): Promise<Message[]> {
  const response = await api.get<ListResponse<Message>>(
    `/v1/sessions/${sessionId}/messages`
  );
  return response.data.data;
}

// Send a user message to a session (triggers workflow)
export async function sendUserMessage(
  sessionId: string,
  content: string,
  controls?: Controls
): Promise<Message> {
  return createMessage(sessionId, {
    message: {
      role: "user",
      content: [{ type: "text", text: content }],
    },
    controls,
  });
}

/** Image attachment info for sending with message */
export interface ImageAttachment {
  imageId: string;
  filename?: string;
}

/**
 * Send a user message with optional image attachments
 */
export async function sendUserMessageWithImages(
  sessionId: string,
  text: string,
  images: ImageAttachment[],
  controls?: Controls
): Promise<Message> {
  const content: Array<{ type: "text"; text: string } | { type: "image_file"; image_id: string; filename?: string }> = [];

  // Add text content if provided
  if (text.trim()) {
    content.push({ type: "text", text: text.trim() });
  }

  // Add image file references
  for (const img of images) {
    content.push({
      type: "image_file",
      image_id: img.imageId,
      filename: img.filename,
    });
  }

  return createMessage(sessionId, {
    message: {
      role: "user",
      content,
    },
    controls,
  });
}
