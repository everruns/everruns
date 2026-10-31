export type ChatEvent = {
  id?: string;
  sequence?: number;
  ts?: string;
  type: string;
  data: Record<string, unknown>;
};
export type Message = { id: string; role: string; text: string };

// Live reconnects append a replay at the end. Sequence order is what separates
// one turn's work from the next.
export function orderedEvents(events: ChatEvent[]): ChatEvent[] {
  return events
    .map((event, index) => ({ event, index }))
    .sort((a, b) => {
      const as = a.event.sequence;
      const bs = b.event.sequence;
      if (typeof as === "number" && typeof bs === "number" && as !== bs) return as - bs;
      return a.index - b.index;
    })
    .map(({ event }) => event);
}
function messageOf(event: ChatEvent): Message | null {
  if (event.type === "conversation.message") {
    const text = typeof event.data.text === "string" ? event.data.text : "";
    const id = event.data.message_id;
    return text.trim() && typeof id === "string" ? { id, role: "assistant", text } : null;
  }
  if (!["input.message", "output.message.completed"].includes(event.type)) return null;
  const data = event.data as {
    message?: { id?: string; role?: string; content?: Array<{ type: string; text?: string }> };
    id?: string;
    role?: string;
    content?: Array<{ type: string; text?: string }>;
  };
  const message = data.message ?? data;
  const text = (message.content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("\n");
  return text
    ? {
        id: message.id ?? event.id ?? "",
        role: message.role ?? (event.type === "input.message" ? "user" : "assistant"),
        text,
      }
    : null;
}
export function conversationMessages(events: ChatEvent[]): Message[] {
  const messages = new Map<string, Message>();
  for (const event of orderedEvents(events)) {
    const message = messageOf(event);
    if (message) {
      messages.set(message.id, message);
      continue;
    }
    if (event.type === "output.message.delta") {
      const id = String(event.data.message_id ?? "");
      if (!id) continue;
      const previous = messages.get(id);
      messages.set(id, {
        id,
        role: "assistant",
        text:
          typeof event.data.accumulated === "string"
            ? event.data.accumulated
            : (previous?.text ?? "") + String(event.data.delta ?? ""),
      });
    }
  }
  return [...messages.values()];
}
