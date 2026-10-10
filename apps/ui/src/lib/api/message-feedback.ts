// Message feedback: the caller's Good / Bad ratings of messages in a session
// (knowledge/ui/chat-experience.md, "Message feedback").

import { api } from "./client";

export type MessageRating = "good" | "bad";

export interface MessageFeedback {
  message_id: string;
  /** Null when the rating was cleared. */
  rating: MessageRating | null;
  comment: string | null;
  updated_at: string;
}

/** The caller's own ratings in a session. */
export async function listMessageFeedback(sessionId: string): Promise<MessageFeedback[]> {
  const response = await api.get<MessageFeedback[]>(`/v1/sessions/${sessionId}/feedback`);
  return response.data;
}

/** Rate a message, or clear the rating with `null`. */
export async function setMessageFeedback(
  sessionId: string,
  messageId: string,
  rating: MessageRating | null,
  comment?: string,
): Promise<MessageFeedback> {
  const response = await api.put<MessageFeedback>(
    `/v1/sessions/${sessionId}/messages/${messageId}/feedback`,
    { rating, comment },
  );
  return response.data;
}
