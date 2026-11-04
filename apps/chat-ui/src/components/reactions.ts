/**
 * Emoji reactions in Work Chat (knowledge/execution/message-reactions.md).
 *
 * Decisions:
 * - Same model as the console, simpler look: chips with counts, and one quick
 *   row of six emoji with no search.
 * - The list comes from the Execution API, which marks the consumer's own
 *   reactions (`me`). Reaction events in the stream only say that something
 *   changed; the newest one's id is the cache key to read the list again.
 */
import type { ChatEvent } from "./transcript";

export const QUICK_REACTIONS = ["👍", "❤️", "😂", "🎉", "👀", "✅"] as const;

export type Reaction = { emoji: string; count: number; me: boolean; agent: boolean };
export type ReactionsByMessage = Map<string, Reaction[]>;

type MessageReactionsRow = { message_id: string; reactions: Reaction[] };

/** Id of the newest reaction event; changes whenever anyone reacts. */
export function reactionVersion(events: ChatEvent[]): string | null {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (event.type.startsWith("message.reaction."))
      return event.id ?? `${event.type}:${event.sequence}`;
  }
  return null;
}

export function reactionsByMessage(rows: MessageReactionsRow[]): ReactionsByMessage {
  return new Map(rows.map((row) => [row.message_id, row.reactions]));
}

/** The map after the consumer adds or removes one reaction, shown before the server answers. */
export function applyReaction(
  map: ReactionsByMessage,
  messageId: string,
  emoji: string,
  add: boolean,
): ReactionsByMessage {
  const current = map.get(messageId) ?? [];
  const existing = current.find((reaction) => reaction.emoji === emoji);
  if (add === Boolean(existing?.me)) return map;
  const next = add
    ? existing
      ? current.map((reaction) =>
          reaction.emoji === emoji ? { ...reaction, me: true, count: reaction.count + 1 } : reaction,
        )
      : [...current, { emoji, count: 1, me: true, agent: false }]
    : current
        .map((reaction) =>
          reaction.emoji === emoji ? { ...reaction, me: false, count: reaction.count - 1 } : reaction,
        )
        .filter((reaction) => reaction.count > 0);
  const out = new Map(map);
  if (next.length) out.set(messageId, next);
  else out.delete(messageId);
  return out;
}

export async function loadReactions(sessionId: string): Promise<ReactionsByMessage> {
  const response = await fetch(`/api/sessions/${sessionId}/reactions`, { cache: "no-store" });
  if (!response.ok) throw new Error("Reactions could not be loaded");
  return reactionsByMessage((await response.json()) as MessageReactionsRow[]);
}

export async function saveReaction(
  sessionId: string,
  messageId: string,
  emoji: string,
  add: boolean,
): Promise<void> {
  const response = await fetch(
    `/api/sessions/${sessionId}/messages/${messageId}/reactions/${encodeURIComponent(emoji)}`,
    { method: add ? "PUT" : "DELETE", cache: "no-store" },
  );
  if (!response.ok) throw new Error("The reaction could not be saved");
}
