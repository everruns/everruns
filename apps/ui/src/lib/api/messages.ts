// Message API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, type RequestOptions } from "./client";
import type { Message, CreateMessageRequest, ListResponse, Controls } from "./types";

export async function createMessage(
  sessionId: string,
  request: CreateMessageRequest,
  options?: RequestOptions,
): Promise<Message> {
  const response = await api.post<Message>(`/v1/sessions/${sessionId}/messages`, request, options);
  return response.data;
}

export async function listMessages(sessionId: string): Promise<Message[]> {
  const response = await api.get<ListResponse<Message>>(`/v1/sessions/${sessionId}/messages`);
  return response.data.data;
}

// Send a user message to a session (triggers workflow).
//
// `addressedParticipantId` optionally targets an active agent participant for
// this turn; when omitted the session host responds (unchanged default).
export async function sendUserMessage(
  sessionId: string,
  content: string,
  controls?: Controls,
  addressedParticipantId?: string | null,
): Promise<Message> {
  return createMessage(sessionId, {
    message: {
      role: "user",
      content: [{ type: "text", text: content }],
    },
    controls,
    ...(addressedParticipantId ? { addressed_participant_id: addressedParticipantId } : {}),
  });
}

/** Image attachment info for sending with message */
export interface ImageAttachment {
  imageId: string;
  filename?: string;
}

export interface FileAttachment {
  fileId: string;
  filename?: string;
}

/**
 * Send a user message with optional image attachments
 */
export async function sendUserMessageWithImages(
  sessionId: string,
  text: string,
  images: ImageAttachment[],
  controls?: Controls,
  addressedParticipantId?: string | null,
  files: FileAttachment[] = [],
): Promise<Message> {
  const content: Array<
    | { type: "text"; text: string }
    | { type: "image_file"; image_id: string; filename?: string }
    | { type: "file"; file_id: string; filename?: string }
  > = [];

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

  // Add file (PDF) references
  for (const f of files) {
    content.push({
      type: "file",
      file_id: f.fileId,
      filename: f.filename,
    });
  }

  return createMessage(sessionId, {
    message: {
      role: "user",
      content,
    },
    controls,
    ...(addressedParticipantId ? { addressed_participant_id: addressedParticipantId } : {}),
  });
}

export interface ChatSendInput {
  /** Client-minted id; resending with the same id returns the stored message. */
  clientMessageId: string;
  text: string;
  images?: ImageAttachment[];
  files?: FileAttachment[];
  controls?: Controls;
  addressedParticipantId?: string | null;
}

/**
 * Send a chat message the turn status row tracks. The client id makes a retry
 * safe: the server answers a repeat with the message it already stored.
 */
export async function sendChatMessage(
  sessionId: string,
  input: ChatSendInput,
  options?: RequestOptions,
): Promise<Message> {
  const content: Array<
    | { type: "text"; text: string }
    | { type: "image_file"; image_id: string; filename?: string }
    | { type: "file"; file_id: string; filename?: string }
  > = [];
  if (input.text.trim()) content.push({ type: "text", text: input.text.trim() });
  for (const img of input.images ?? []) {
    content.push({ type: "image_file", image_id: img.imageId, filename: img.filename });
  }
  for (const f of input.files ?? []) {
    content.push({ type: "file", file_id: f.fileId, filename: f.filename });
  }
  return createMessage(
    sessionId,
    {
      message: { role: "user", content },
      controls: input.controls,
      client_message_id: input.clientMessageId,
      ...(input.addressedParticipantId
        ? { addressed_participant_id: input.addressedParticipantId }
        : {}),
    },
    options,
  );
}
