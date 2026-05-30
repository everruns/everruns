import type { Event } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";

export function getKnownTurnId(event: Event): string | undefined {
  const reasonItemData = getEventData(event, "reason.item");
  return reasonItemData?.turn_id ?? event.context?.turn_id;
}

export function isStructuralWorkLogEvent(event: Event): boolean {
  return event.type === "act.started" || event.type === "tool.call_requested";
}

export function hasReasoningWorkLogSummary(event: Event): boolean {
  const reasonItemData = getEventData(event, "reason.item");
  if (reasonItemData?.summary?.some((item) => item.trim().length > 0)) return true;

  const reasonCompletedData = getEventData(event, "reason.completed");
  return !!reasonCompletedData?.text_preview;
}

export function shouldRenderWorkLogEvent(
  event: Event,
  turnIterationsByTurnId: Map<string, number>,
  structuralWorkTurnIds: Set<string>,
  reasoningMultiIterationTurnIds: Set<string>,
): boolean {
  if (isStructuralWorkLogEvent(event)) return true;
  if (!hasReasoningWorkLogSummary(event)) return false;

  const turnId = getKnownTurnId(event);
  if (!turnId) return false;

  return (
    (turnIterationsByTurnId.get(turnId) ?? 0) > 1 ||
    structuralWorkTurnIds.has(turnId) ||
    reasoningMultiIterationTurnIds.has(turnId)
  );
}

export function getReasoningMultiIterationTurnIds(events: Event[]): Set<string> {
  const countsByTurnId = new Map<string, number>();
  const multiIterationTurnIds = new Set<string>();

  for (const event of events) {
    const reasonCompletedData = getEventData(event, "reason.completed");
    if (!reasonCompletedData) continue;

    const turnId = getKnownTurnId(event);
    if (!turnId) continue;

    const count = (countsByTurnId.get(turnId) ?? 0) + 1;
    countsByTurnId.set(turnId, count);
    if (count > 1) multiIterationTurnIds.add(turnId);
  }

  return multiIterationTurnIds;
}
