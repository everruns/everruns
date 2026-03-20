// Important decision: derive completed-turn dividers from raw SSE events so
// optimistic input rows never show a fake duration and client tool requests
// can take over as the last visible item when embedded tool calls are hidden.

import type { Event } from "@/lib/api/types";
import {
  getTextFromContent,
  getToolCallsFromContent,
  isImageFilePart,
  getEventData,
} from "@/lib/api/types";

function getClientRequestedToolCallIds(events: Event[]): Set<string> {
  const ids = new Set<string>();

  for (const event of events) {
    const data = getEventData(event, "tool.call_requested");
    if (!data) continue;
    for (const toolCall of data.tool_calls ?? []) {
      ids.add(toolCall.id);
    }
  }

  return ids;
}

function getInputMessageTurnIds(events: Event[]): Map<string, string> {
  const turnIds = new Map<string, string>();

  for (const event of events) {
    const data = getEventData(event, "turn.started");
    if (!data) continue;
    turnIds.set(data.input_message_id, data.turn_id);
  }

  return turnIds;
}

function isVisibleChatEvent(event: Event, clientRequestedToolCallIds: Set<string>): boolean {
  if (event.type === "input.message") {
    return true;
  }

  const reqData = getEventData(event, "tool.call_requested");
  if (reqData) {
    return (reqData.tool_calls?.length ?? 0) > 0;
  }

  const actData = getEventData(event, "act.started");
  if (actData) {
    return (actData.tool_calls?.length ?? 0) > 0;
  }

  const outData = getEventData(event, "output.message.completed");
  if (!outData) {
    return false;
  }

  const content = outData.message?.content ?? [];
  const text = getTextFromContent(content);
  const hasImages = content.some(isImageFilePart);
  const visibleToolCalls = getToolCallsFromContent(content).filter(
    (toolCall) => !clientRequestedToolCallIds.has(toolCall.id),
  );

  return !!text || hasImages || visibleToolCalls.length > 0;
}

function getEventTurnId(
  event: Event,
  inputMessageTurnIds: Map<string, string>,
): string | undefined {
  if (event.context.turn_id) {
    return event.context.turn_id;
  }

  const data = getEventData(event, "input.message");
  if (!data) return undefined;
  return data.message?.id ? inputMessageTurnIds.get(data.message.id) : undefined;
}

export function getCompletedTurnDurationsByEvent(events: Event[]): Map<string, number> {
  const clientRequestedToolCallIds = getClientRequestedToolCallIds(events);
  const inputMessageTurnIds = getInputMessageTurnIds(events);
  const lastVisibleEventIdByTurn = new Map<string, string>();
  const durationMsByTurn = new Map<string, number>();

  for (const event of events) {
    if (isVisibleChatEvent(event, clientRequestedToolCallIds)) {
      const turnId = getEventTurnId(event, inputMessageTurnIds);
      if (turnId) {
        lastVisibleEventIdByTurn.set(turnId, event.id);
      }
    }

    const tcData = getEventData(event, "turn.completed");
    if (tcData && tcData.duration_ms != null) {
      durationMsByTurn.set(tcData.turn_id, tcData.duration_ms);
    }
  }

  const durationsByEvent = new Map<string, number>();

  for (const [turnId, durationMs] of durationMsByTurn) {
    const eventId = lastVisibleEventIdByTurn.get(turnId);
    if (eventId) {
      durationsByEvent.set(eventId, durationMs);
    }
  }

  return durationsByEvent;
}

export function formatWorkedDuration(durationMs: number): string {
  if (durationMs < 1000) {
    return `${durationMs}ms`;
  }

  const totalSeconds = Math.round(durationMs / 1000);

  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }

  const totalMinutes = Math.floor(totalSeconds / 60);
  const remainingSeconds = totalSeconds % 60;

  if (totalMinutes < 60) {
    return remainingSeconds > 0 ? `${totalMinutes}m ${remainingSeconds}s` : `${totalMinutes}m`;
  }

  const hours = Math.floor(totalMinutes / 60);
  const remainingMinutes = totalMinutes % 60;

  return remainingMinutes > 0 ? `${hours}h ${remainingMinutes}m` : `${hours}h`;
}
