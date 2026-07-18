import type { Event } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";

export interface StreamingMessageState {
  messageId: string | null;
  turnId: string | null;
  text: string | null;
  isThinking: boolean;
  iteration: number | null;
}

const EMPTY_STREAMING_MESSAGE: StreamingMessageState = {
  messageId: null,
  turnId: null,
  text: null,
  isThinking: false,
  iteration: null,
};

function startedForMessage(events: Event[], before: number, messageId: string) {
  for (let i = before; i >= 0; i--) {
    const started = getEventData(events[i], "output.message.started");
    if (started?.message_id === messageId) return started;
  }
  return null;
}

function startedForTurn(events: Event[], before: number, turnId: string) {
  for (let i = before; i >= 0; i--) {
    const started = getEventData(events[i], "output.message.started");
    if (started?.turn_id === turnId) return started;
  }
  return null;
}

/**
 * Projects the latest active assistant-message lifecycle from an event stream.
 * Message identity is the grouping key; a turn can contain multiple commentary
 * and final-answer messages with independent accumulated text.
 */
export function latestStreamingMessage(events: Event[]): StreamingMessageState {
  for (let i = events.length - 1; i >= 0; i--) {
    const event = events[i];

    if (
      event.type === "session.idled" ||
      event.type === "turn.completed" ||
      event.type === "turn.failed" ||
      event.type === "turn.cancelled" ||
      event.type === "output.message.completed"
    ) {
      return EMPTY_STREAMING_MESSAGE;
    }

    const replaced = getEventData(event, "output.message.replaced");
    if (replaced) {
      const started = startedForMessage(events, i - 1, replaced.message_id);
      return {
        messageId: replaced.message_id,
        turnId: replaced.turn_id,
        text: replaced.replacement,
        isThinking: false,
        iteration: started?.iteration ?? null,
      };
    }

    const delta = getEventData(event, "output.message.delta");
    if (delta) {
      const started = startedForMessage(events, i - 1, delta.message_id);
      return {
        messageId: delta.message_id,
        turnId: delta.turn_id,
        text: delta.accumulated,
        isThinking: false,
        iteration: started?.iteration ?? null,
      };
    }

    const started = getEventData(event, "output.message.started");
    if (started) {
      return {
        messageId: started.message_id,
        turnId: started.turn_id,
        text: null,
        isThinking: false,
        iteration: started.iteration ?? null,
      };
    }

    const thinking = getEventData(event, "reason.thinking.started");
    if (thinking) {
      const message = startedForTurn(events, i - 1, thinking.turn_id);
      return {
        messageId: message?.message_id ?? null,
        turnId: thinking.turn_id,
        text: null,
        isThinking: true,
        iteration: message?.iteration ?? null,
      };
    }
  }

  return EMPTY_STREAMING_MESSAGE;
}
