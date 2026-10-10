// Message feedback: the caller's Good / Bad ratings of messages in a session
// (knowledge/ui/chat-experience.md, "Message feedback").

import { api } from "./client";
import type { MessageFeedback, MessageRating } from "./schema-types";

export type { MessageFeedback, MessageRating };

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
