// Emoji reactions on the messages of a session, from people and from the
// agent (knowledge/execution/message-reactions.md).

import { api } from "./client";
import type {
  MessageReactions,
  ReactionActorView,
  ReactionChange,
  ReactionSummary,
} from "./schema-types";

export type { MessageReactions, ReactionActorView, ReactionChange, ReactionSummary };

/** Quick picks offered by the reaction picker, in order. */
export const QUICK_REACTIONS = ["👍", "❤️", "😂", "🎉", "👀", "✅", "❌", "🙏"] as const;

/** Reactions on the messages of a session, with `me` marking the caller's own. */
export async function listMessageReactions(sessionId: string): Promise<MessageReactions[]> {
  const response = await api.get<MessageReactions[]>(`/v1/sessions/${sessionId}/reactions`);
  return response.data;
}

/** Put the caller's reaction on a message, or take it off. */
export async function setMessageReaction(
  sessionId: string,
  messageId: string,
  emoji: string,
  add: boolean,
): Promise<ReactionChange> {
  const path = `/v1/sessions/${sessionId}/messages/${messageId}/reactions/${encodeURIComponent(emoji)}`;
  const response = add
    ? await api.put<ReactionChange>(path)
    : await api.delete<ReactionChange>(path);
  return response.data;
}

/**
 * The list after the caller adds or removes one reaction, for an optimistic
 * update. The caller shows as "You" until the server's answer replaces it.
 */
export function applyReaction(
  messages: MessageReactions[],
  messageId: string,
  emoji: string,
  add: boolean,
): MessageReactions[] {
  const others = messages.filter((row) => row.message_id !== messageId);
  const current = messages.find((row) => row.message_id === messageId)?.reactions ?? [];
  const existing = current.find((reaction) => reaction.emoji === emoji);
  let reactions: ReactionSummary[];
  if (add) {
    if (existing?.me) return messages;
    reactions = existing
      ? current.map((reaction) =>
          reaction.emoji === emoji
            ? { ...reaction, me: true, count: reaction.count + 1 }
            : reaction,
        )
      : [...current, { emoji, count: 1, me: true, agent: false, actors: [] }];
  } else {
    if (!existing?.me) return messages;
    reactions = current
      .map((reaction) =>
        reaction.emoji === emoji ? { ...reaction, me: false, count: reaction.count - 1 } : reaction,
      )
      .filter((reaction) => reaction.count > 0);
  }
  return reactions.length > 0 ? [...others, { message_id: messageId, reactions }] : others;
}
