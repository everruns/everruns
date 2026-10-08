import type { Event } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";
import { getRuntimeErrorFromOutputMessage } from "@/lib/runtime-errors";

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

/**
 * Commentary is intermediate assistant text, shown with tool calls inside the
 * work log. A derived phase only means the message called tools, so a text-only
 * derived message stays a normal answer: it cannot be told apart from a final
 * answer. Errors and images stay on the message row.
 */
export function isCommentaryWorkLogEvent(event: Event): boolean {
  const output = getEventData(event, "output.message.completed");
  if (!output?.message || output.message.phase !== "commentary") return false;
  if (getRuntimeErrorFromOutputMessage(output)) return false;
  if (output.message.content?.some((part) => part.type === "image" || part.type === "image_file")) {
    return false;
  }
  // phase_source is on the wire message. The legacy Message type is size-capped.
  const phaseSource = (output.message as { phase_source?: string }).phase_source;
  if (phaseSource === "derived") {
    return output.message.content?.some((part) => part.type === "tool_call") ?? false;
  }
  return true;
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
