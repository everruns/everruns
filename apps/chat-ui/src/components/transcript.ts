export type ChatEvent = {
  id?: string;
  sequence?: number;
  ts?: string;
  type: string;
  data: Record<string, unknown>;
};
/**
 * A message the platform wrote, not the person: a schedule run, a trigger, a
 * thread update, a notice. `count` is how many such messages in a row from the
 * same source this one stands for (knowledge/execution/message-origin.md).
 */
export type MessageOrigin = { kind: string; name?: string; count: number };
export type Message = { id: string; role: string; text: string; origin?: MessageOrigin };

const ORIGIN_LABELS: Record<string, string> = {
  schedule: "Scheduled check",
  channel_schedule: "Scheduled check",
  agent_trigger: "Trigger",
  webhook: "Webhook",
  task_wake: "Update",
  system: "Notice",
};

/** The one line a consumer sees for a platform message: kind, name, count. */
export function originLabel(origin: MessageOrigin): string {
  const parts = [ORIGIN_LABELS[origin.kind] ?? "Automatic message"];
  if (origin.name) parts.push(origin.name);
  if (origin.count > 1) parts.push(`${origin.count}×`);
  return parts.join(" · ");
}

// The server normalizes every origin marker into one `everruns_origin` object
// on public metadata; ids are not shown to consumers.
function originOf(metadata: unknown): MessageOrigin | undefined {
  if (!metadata || typeof metadata !== "object") return undefined;
  const origin = (metadata as { everruns_origin?: unknown }).everruns_origin;
  if (!origin || typeof origin !== "object") return undefined;
  const { kind, name } = origin as { kind?: unknown; name?: unknown };
  if (typeof kind !== "string" || !(kind in ORIGIN_LABELS)) return undefined;
  return { kind, name: typeof name === "string" && name.trim() ? name : undefined, count: 1 };
}

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
    message?: {
      id?: string;
      role?: string;
      content?: Array<{ type: string; text?: string }>;
      metadata?: unknown;
    };
    id?: string;
    role?: string;
    content?: Array<{ type: string; text?: string }>;
  };
  const message: {
    id?: string;
    role?: string;
    content?: Array<{ type: string; text?: string }>;
    metadata?: unknown;
  } = data.message ?? data;
  const text = (message.content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("\n");
  return text
    ? {
        id: message.id ?? event.id ?? "",
        role: message.role ?? (event.type === "input.message" ? "user" : "assistant"),
        text,
        ...(event.type === "input.message" && originOf(message.metadata)
          ? { origin: originOf(message.metadata) }
          : {}),
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
  return collapseSilentRuns([...messages.values()]);
}

/**
 * Platform messages from the same source with nothing between them, such as a
 * schedule polling every five minutes with no reply, read as one line with a
 * count. The latest keeps its id, so its live work still attaches to it.
 */
function collapseSilentRuns(messages: Message[]): Message[] {
  const out: Message[] = [];
  for (const message of messages) {
    const previous = out.at(-1);
    if (
      message.origin &&
      previous?.origin &&
      previous.origin.kind === message.origin.kind &&
      previous.origin.name === message.origin.name
    ) {
      out[out.length - 1] = {
        ...message,
        origin: { ...message.origin, count: previous.origin.count + message.origin.count },
      };
      continue;
    }
    out.push(message);
  }
  return out;
}
