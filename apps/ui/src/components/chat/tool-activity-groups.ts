import type { Event, ToolCallSummary, ToolCompletedData } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";

/**
 * Synthetic tool calls the backend emits to ask the user something directly.
 * They get their own inline card, so the generic activity timeline leaves them
 * alone rather than listing them as anonymous pending tool calls.
 */
export const INTERACTIVE_TOOL_CALLS = new Set(["setup_connection", "confirm_url_elicitation"]);

export interface TimelineToolRow {
  id: string;
  label: string;
  completedLabel?: string;
  result?: ToolCompletedData;
  state: "running" | "waiting" | "completed" | "error";
}

export interface ToolActivityEventGroup {
  key: string;
  anchorEventId: string;
  headline: string;
  completedHeadline?: string;
  rows: TimelineToolRow[];
}

export interface ToolActivityGroups {
  byAnchorEventId: Map<string, ToolActivityEventGroup>;
  groupedEventIds: Set<string>;
  narratedToolCallIds: Set<string>;
}

function fallbackToolLabel(name: string | undefined): string {
  if (!name) return "Tool call";
  return name
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

/** Fold durable activity lifecycle events into one stable group per exec/request batch. */
export function buildToolActivityGroups(events: Event[], workingLabel: string): ToolActivityGroups {
  const groupsByKey = new Map<string, ToolActivityEventGroup>();
  const groupKeyByToolCallId = new Map<string, string>();
  const groupedEventIds = new Set<string>();
  const narratedToolCallIds = new Set<string>();
  const seenEventIds = new Set<string>();

  const ensureGroup = (key: string, event: Event, headline?: string) => {
    const existing = groupsByKey.get(key);
    if (existing) return existing;
    const group: ToolActivityEventGroup = {
      key,
      anchorEventId: event.id,
      headline: headline ?? workingLabel,
      rows: [],
    };
    groupsByKey.set(key, group);
    return group;
  };

  const ensureRow = (
    group: ToolActivityEventGroup,
    id: string,
    label: string,
    state: TimelineToolRow["state"],
  ) => {
    const existing = group.rows.find((row) => row.id === id);
    if (existing) {
      existing.state = state;
      return existing;
    }
    const row: TimelineToolRow = { id, label, state };
    group.rows.push(row);
    return row;
  };

  const addSummary = (
    group: ToolActivityEventGroup,
    summary: ToolCallSummary,
    state: TimelineToolRow["state"],
  ) => {
    const row = ensureRow(
      group,
      summary.id,
      summary.narration ?? summary.display_name ?? fallbackToolLabel(summary.name),
      state,
    );
    row.completedLabel = summary.completed_narration ?? row.completedLabel;
    groupKeyByToolCallId.set(summary.id, group.key);
    narratedToolCallIds.add(summary.id);
  };

  for (const event of events) {
    if (seenEventIds.has(event.id)) continue;
    seenEventIds.add(event.id);

    const execKey = event.context?.exec_id ? `exec:${event.context.exec_id}` : undefined;
    const actStarted = getEventData(event, "act.started");
    if (actStarted) {
      const group = ensureGroup(execKey ?? `act:${event.id}`, event, actStarted.headline);
      if (actStarted.headline) group.headline = actStarted.headline;
      for (const summary of actStarted.tool_calls ?? []) addSummary(group, summary, "running");
      groupedEventIds.add(event.id);
      continue;
    }

    const request = getEventData(event, "tool.call_requested");
    if (request) {
      const summariesById = new Map(
        (request.tool_summaries ?? []).map((summary) => [summary.id, summary]),
      );
      const genericCalls = request.tool_calls.filter(
        (call) => !INTERACTIVE_TOOL_CALLS.has(call.name),
      );
      if (genericCalls.length === 0) continue;

      const existingKey = genericCalls
        .map((call) => groupKeyByToolCallId.get(call.id))
        .find((key): key is string => !!key);
      const group = ensureGroup(
        existingKey ?? execKey ?? `request:${event.id}`,
        event,
        request.headline,
      );
      if (request.headline && group.rows.length === 0) group.headline = request.headline;
      if (!group.completedHeadline && request.completed_headline) {
        group.completedHeadline = request.completed_headline;
      }
      for (const call of genericCalls) {
        addSummary(
          group,
          summariesById.get(call.id) ?? { id: call.id, name: call.name },
          "waiting",
        );
      }
      // Keep mixed request events renderable so specialized cards (for example,
      // setup_connection or confirm_url_elicitation) can appear alongside the
      // generic activity group.
      if (genericCalls.length === request.tool_calls.length) groupedEventIds.add(event.id);
      continue;
    }

    const toolStarted = getEventData(event, "tool.started");
    if (toolStarted) {
      const id = toolStarted.tool_call.id;
      const key = groupKeyByToolCallId.get(id) ?? execKey ?? `tool:${id}`;
      const group = ensureGroup(key, event, toolStarted.narration);
      const row = ensureRow(
        group,
        id,
        toolStarted.narration ??
          toolStarted.display_name ??
          fallbackToolLabel(toolStarted.tool_call.name),
        "running",
      );
      if (toolStarted.narration) row.label = toolStarted.narration;
      groupKeyByToolCallId.set(id, key);
      narratedToolCallIds.add(id);
      groupedEventIds.add(event.id);
      continue;
    }

    const progress = getEventData(event, "tool.progress");
    if (progress) {
      const id = progress.tool_call_id;
      const key = groupKeyByToolCallId.get(id) ?? execKey ?? `tool:${id}`;
      const group = ensureGroup(key, event, progress.message);
      const row = ensureRow(
        group,
        id,
        progress.message || progress.display_name || fallbackToolLabel(progress.tool_name),
        "running",
      );
      if (progress.message) row.label = progress.message;
      groupKeyByToolCallId.set(id, key);
      narratedToolCallIds.add(id);
      groupedEventIds.add(event.id);
      continue;
    }

    const toolCompleted = getEventData(event, "tool.completed");
    if (toolCompleted) {
      const id = toolCompleted.tool_call_id;
      const key = groupKeyByToolCallId.get(id) ?? execKey ?? `tool:${id}`;
      const group = ensureGroup(key, event, toolCompleted.narration);
      const row = ensureRow(
        group,
        id,
        toolCompleted.narration ??
          toolCompleted.display_name ??
          fallbackToolLabel(toolCompleted.tool_name),
        toolCompleted.success ? "completed" : "error",
      );
      row.label = toolCompleted.narration ?? row.completedLabel ?? row.label;
      row.result = toolCompleted;
      groupKeyByToolCallId.set(id, key);
      narratedToolCallIds.add(id);
      groupedEventIds.add(event.id);
      continue;
    }

    const actCompleted = getEventData(event, "act.completed");
    if (actCompleted && execKey) {
      const group = groupsByKey.get(execKey);
      if (group) {
        group.completedHeadline = actCompleted.headline ?? group.completedHeadline;
        groupedEventIds.add(event.id);
      }
    }
  }

  const byAnchorEventId = new Map<string, ToolActivityEventGroup>();
  for (const group of groupsByKey.values()) {
    if (group.rows.length > 0) byAnchorEventId.set(group.anchorEventId, group);
  }
  return { byAnchorEventId, groupedEventIds, narratedToolCallIds };
}
