import type { Event } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";

export function getKnownTurnId(event: Event): string | undefined {
  const reasonItemData = getEventData(event, "reason.item");
  return reasonItemData?.turn_id ?? event.context?.turn_id;
}

export function isStructuralWorkLogEvent(event: Event): boolean {
  return (
    event.type === "act.started" ||
    event.type === "act.completed" ||
    event.type === "tool.started" ||
    event.type === "tool.progress" ||
    event.type === "tool.completed" ||
    event.type === "tool.call_requested" ||
    event.type === "tool.hosted_call"
  );
}

export function hasReasoningWorkLogSummary(event: Event): boolean {
  // reason.completed previews assistant output; only reason.item carries
  // provider-curated reasoning. Rendering the preview repeats the answer.
  const reasonItemData = getEventData(event, "reason.item");
  return !!reasonItemData?.summary?.some((item) => item.trim().length > 0);
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
