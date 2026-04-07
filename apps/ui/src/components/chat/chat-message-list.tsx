/**
 * Decisions:
 * - Keep event-to-component routing in one place so the transcript stays easy to extend.
 * - Tool-only assistant events share the same grouping rules as explicit tool-call-requested events.
 * - Narrated act timelines suppress duplicate tool groups to avoid repeated progress chrome.
 */
"use client";

import { Bot, CalendarClock, Loader2 } from "lucide-react";
import { memo, useMemo } from "react";
import type {
  ContentPart,
  Event,
  InputMessageData,
  OutputMessageCompletedData,
  ToolCallSummary,
  ToolCompletedData,
  ToolProgressData,
} from "@/lib/api/types";
import type { ToolOutputStreams } from "@/app/(main)/sessions/[sessionId]/session-context";
import { getEventData, isImageFilePart } from "@/lib/api/types";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { MessageImage } from "@/components/chat/image-attachments";
import { MessageContent } from "@/components/chat/message-content";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { SetupConnectionToolCall } from "@/components/chat/setup-connection-tool-call";
import {
  ToolActivityTimelineGroup,
  type TimelineToolRow,
} from "@/components/chat/tool-activity-timeline-group";
import {
  formatWorkedDuration,
  getCompletedTurnDurationsByEvent,
} from "@/components/chat/turn-delimiter";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { CompactionDivider } from "@/components/chat/compaction-divider";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";
import { useLocale } from "@/providers/locale-provider";

interface ChatMessageListProps {
  events: Event[] | undefined;
  chatEvents: Event[];
  sessionId: string;
  toolResultsMap: Map<string, ToolCompletedData>;
  toolProgressMap: Map<string, ToolProgressData>;
  toolOutputMap: Map<string, ToolOutputStreams>;
  eventsLoading: boolean;
  hasMoreEvents: boolean;
  loadingOlderEvents: boolean;
  getMessageText: (data: InputMessageData | OutputMessageCompletedData) => string;
  getToolCalls: (data: OutputMessageCompletedData) => ToolCallContent[];
}

type ActGroup = {
  startEventId: string;
  headline: string;
  completedHeadline?: string;
  rows: TimelineToolRow[];
};

function getMessageImages(content: ContentPart[]): Array<{ image_id: string; filename?: string }> {
  return content.filter(isImageFilePart).map((part) => ({
    image_id: part.image_id,
    filename: part.filename,
  }));
}

function buildActGroups(chatEvents: Event[], workingLabel: string) {
  const groupsByExecId = new Map<string, ActGroup>();

  const ensureRow = (
    group: ActGroup,
    id: string,
    fallbackLabel: string,
    state: TimelineToolRow["state"],
  ) => {
    const existing = group.rows.find((row) => row.id === id);
    if (existing) {
      existing.label = existing.label || fallbackLabel;
      existing.state = state;
      return existing;
    }

    const row: TimelineToolRow = { id, label: fallbackLabel, state };
    group.rows.push(row);
    return row;
  };

  for (const event of chatEvents) {
    const execId = event.context?.exec_id;
    if (!execId) continue;

    const actStarted = getEventData(event, "act.started");
    if (actStarted) {
      groupsByExecId.set(execId, {
        startEventId: event.id,
        headline: actStarted.headline ?? workingLabel,
        rows: (actStarted.tool_calls ?? []).map((toolCall: ToolCallSummary) => ({
          id: toolCall.id,
          label: toolCall.narration ?? toolCall.display_name ?? toolCall.name,
          state: "running",
        })),
      });
      continue;
    }

    const group = groupsByExecId.get(execId);
    if (!group) continue;

    const toolStarted = getEventData(event, "tool.started");
    if (toolStarted) {
      const row = ensureRow(
        group,
        toolStarted.tool_call.id,
        toolStarted.narration ?? toolStarted.display_name ?? toolStarted.tool_call.name,
        "running",
      );
      row.label = toolStarted.narration ?? row.label;
      continue;
    }

    const toolCompleted = getEventData(event, "tool.completed");
    if (toolCompleted) {
      const row = ensureRow(
        group,
        toolCompleted.tool_call_id,
        toolCompleted.narration ??
          toolCompleted.display_name ??
          toolCompleted.tool_name ??
          "Tool call",
        toolCompleted.success ? "completed" : "error",
      );
      row.label = toolCompleted.narration ?? row.label;
      row.result = toolCompleted;
      row.state = toolCompleted.success ? "completed" : "error";
      continue;
    }

    const actCompleted = getEventData(event, "act.completed");
    if (actCompleted) {
      group.completedHeadline = actCompleted.headline;
    }
  }

  return new Map(Array.from(groupsByExecId.values()).map((group) => [group.startEventId, group]));
}

function renderTurnDivider(
  eventId: string,
  turnDurationByEventId: Map<string, number>,
  workedForText: string | null,
) {
  const durationMs = turnDurationByEventId.get(eventId);
  if (durationMs == null || !workedForText) return null;

  return (
    <div className="flex items-center gap-4 pt-3 text-xs font-medium text-muted-foreground sm:text-sm">
      <div className="h-px flex-1 bg-border" />
      <span className="whitespace-nowrap">{workedForText}</span>
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}

export const ChatMessageList = memo(function ChatMessageList({
  events,
  chatEvents,
  sessionId,
  toolResultsMap,
  toolProgressMap,
  toolOutputMap,
  eventsLoading,
  hasMoreEvents,
  loadingOlderEvents,
  getMessageText,
  getToolCalls,
}: ChatMessageListProps) {
  const { t } = useLocale();
  const clientRequestedToolCallIds = useMemo(() => {
    const ids = new Set<string>();
    for (const event of chatEvents) {
      const data = getEventData(event, "tool.call_requested");
      if (!data) continue;
      for (const toolCall of data.tool_calls ?? []) {
        ids.add(toolCall.id);
      }
    }
    return ids;
  }, [chatEvents]);

  const turnDurationByEventId = useMemo(
    () => getCompletedTurnDurationsByEvent(events ?? []),
    [events],
  );
  const hasNarratedActEvents = useMemo(
    () => chatEvents.some((event) => event.type === "act.started"),
    [chatEvents],
  );
  const actGroupsByStartEventId = useMemo(
    () => buildActGroups(chatEvents, t("working")),
    [chatEvents, t],
  );

  if (eventsLoading) {
    return (
      <div className="space-y-4">
        <div className="ml-auto h-20 w-3/4 animate-pulse rounded-md bg-muted" />
        <div className="h-20 w-3/4 animate-pulse rounded-md bg-muted" />
        <div className="h-20 w-2/3 animate-pulse rounded-md bg-muted" />
      </div>
    );
  }

  if (chatEvents.length === 0) {
    return (
      <div className="flex flex-col items-center justify-end text-center text-muted-foreground">
        <div className={chatSurfaceStyles.emptyStateCard}>
          <div className="mx-auto mb-4 flex h-10 w-10 items-center justify-center border border-border/70 bg-background text-muted-foreground">
            <Bot className="h-5 w-5 opacity-65" />
          </div>
          <p className="text-lg font-medium text-foreground">{t("no_messages_yet")}</p>
          <p className="mt-1 text-sm">{t("start_with_prompt")}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {loadingOlderEvents && hasMoreEvents && (
        <div className="flex items-center justify-center py-3">
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          <span className="ml-2 text-xs text-muted-foreground">{t("loading_older_messages")}</span>
        </div>
      )}
      {chatEvents.map((event) => {
        if (
          event.type === "tool.started" ||
          event.type === "tool.completed" ||
          event.type === "act.completed"
        ) {
          return null;
        }

        if (event.type === "context.compacted") {
          const compactedData = getEventData(event, "context.compacted");
          return compactedData ? <CompactionDivider key={event.id} data={compactedData} /> : null;
        }

        if (event.type === "act.started") {
          const group = actGroupsByStartEventId.get(event.id);
          if (!group || group.rows.length === 0) return null;
          return (
            <div key={event.id} className="space-y-3">
              <div className="ml-9 space-y-1">
                <ToolActivityTimelineGroup
                  headline={group.headline}
                  completedHeadline={group.completedHeadline}
                  rows={group.rows}
                />
              </div>
              {renderTurnDivider(
                event.id,
                turnDurationByEventId,
                t("worked_for", {
                  duration: formatWorkedDuration(turnDurationByEventId.get(event.id) ?? 0),
                }),
              )}
            </div>
          );
        }

        if (event.type === "tool.call_requested") {
          const reqData = getEventData(event, "tool.call_requested");
          if (!reqData?.tool_calls?.length) return null;

          if (reqData.headline || reqData.tool_summaries?.length) {
            const rows: TimelineToolRow[] = (reqData.tool_summaries ?? []).map((summary) => ({
              id: summary.id,
              label: summary.narration ?? summary.display_name ?? summary.name,
              state: "waiting",
            }));

            if (rows.length > 0) {
              return (
                <div key={event.id} className="space-y-3">
                  <div className="ml-9 space-y-1">
                    <ToolActivityTimelineGroup
                      headline={reqData.headline ?? t("waiting_on_tools")}
                      rows={rows}
                    />
                  </div>
                  {renderTurnDivider(
                    event.id,
                    turnDurationByEventId,
                    t("worked_for", {
                      duration: formatWorkedDuration(turnDurationByEventId.get(event.id) ?? 0),
                    }),
                  )}
                </div>
              );
            }
          }

          const connectionCalls = reqData.tool_calls.filter(
            (toolCall) => toolCall.name === "setup_connection",
          );
          const otherCalls = reqData.tool_calls.filter(
            (toolCall) => toolCall.name !== "setup_connection",
          );

          return (
            <div key={event.id} className="space-y-3">
              <div className="ml-9 space-y-1">
                {connectionCalls.map((toolCall) => (
                  <SetupConnectionToolCall
                    key={toolCall.id}
                    sessionId={sessionId}
                    toolCallId={toolCall.id}
                    provider={(toolCall.arguments as { provider?: string })?.provider ?? "unknown"}
                    toolResultsMap={toolResultsMap}
                  />
                ))}
                {otherCalls.length > 0 && (
                  <ToolActivityGroup
                    toolCalls={otherCalls}
                    toolResultsMap={toolResultsMap}
                    toolProgressMap={toolProgressMap}
                    toolOutputMap={toolOutputMap}
                    mode="client"
                  />
                )}
              </div>
              {renderTurnDivider(
                event.id,
                turnDurationByEventId,
                t("worked_for", {
                  duration: formatWorkedDuration(turnDurationByEventId.get(event.id) ?? 0),
                }),
              )}
            </div>
          );
        }

        const isUser = event.type === "input.message";
        const inputData = getEventData(event, "input.message");
        const outputData = getEventData(event, "output.message.completed");
        const data = inputData ?? outputData;
        if (!data) return null;

        const textContent = getMessageText(data);
        const toolCalls = isUser
          ? []
          : outputData
            ? getToolCalls(outputData).filter(
                (toolCall) => !clientRequestedToolCallIds.has(toolCall.id),
              )
            : [];
        const images = data.message?.content ? getMessageImages(data.message.content) : [];
        const isScheduleTriggered = isUser && data.message?.metadata?.source === "schedule";
        const isToolOnlyMessage =
          !isUser && toolCalls.length > 0 && !textContent && images.length === 0;

        if (isToolOnlyMessage) {
          if (hasNarratedActEvents) return null;
          return (
            <div key={event.id} className="space-y-3">
              <div className="ml-9 space-y-1">
                <ToolActivityGroup
                  toolCalls={toolCalls}
                  toolResultsMap={toolResultsMap}
                  toolProgressMap={toolProgressMap}
                  toolOutputMap={toolOutputMap}
                />
              </div>
              {renderTurnDivider(
                event.id,
                turnDurationByEventId,
                t("worked_for", {
                  duration: formatWorkedDuration(turnDurationByEventId.get(event.id) ?? 0),
                }),
              )}
            </div>
          );
        }

        return (
          <div key={event.id} className="space-y-2">
            {(textContent || images.length > 0) && (
              <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                {isUser ? (
                  <div className={chatSurfaceStyles.userMessage}>
                    {isScheduleTriggered && (
                      <div className="mb-1 flex items-center gap-1 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                        <CalendarClock className="h-3 w-3" />
                        <span>{t("scheduled")}</span>
                      </div>
                    )}
                    <div className="flex items-start gap-2">
                      <div className="flex-1 space-y-2">
                        {textContent && <p className="whitespace-pre-wrap">{textContent}</p>}
                        {images.length > 0 && (
                          <div className="mt-2 flex flex-wrap gap-2">
                            {images.map((image) => (
                              <MessageImage
                                key={image.image_id}
                                imageId={image.image_id}
                                filename={image.filename}
                              />
                            ))}
                          </div>
                        )}
                      </div>
                      <MessageInfoIcon event={event} />
                    </div>
                  </div>
                ) : (
                  <div className={chatSurfaceStyles.agentMessageRow}>
                    <div className={chatSurfaceStyles.agentIcon}>
                      <Bot className="h-3.5 w-3.5" />
                    </div>
                    <div className="flex flex-1 items-start gap-2">
                      <div className={chatSurfaceStyles.agentMessage}>
                        {textContent && <MessageContent text={textContent} />}
                        {images.length > 0 && (
                          <div className="mt-2 flex flex-wrap gap-2">
                            {images.map((image) => (
                              <MessageImage
                                key={image.image_id}
                                imageId={image.image_id}
                                filename={image.filename}
                              />
                            ))}
                          </div>
                        )}
                      </div>
                      <MessageInfoIcon event={event} />
                    </div>
                  </div>
                )}
              </div>
            )}

            {toolCalls.length > 0 && !hasNarratedActEvents && (
              <div className="ml-9 space-y-1">
                <ToolActivityGroup
                  toolCalls={toolCalls}
                  toolResultsMap={toolResultsMap}
                  toolProgressMap={toolProgressMap}
                  toolOutputMap={toolOutputMap}
                />
              </div>
            )}

            {renderTurnDivider(event.id, turnDurationByEventId, t("worked_for"))}
          </div>
        );
      })}
    </div>
  );
});
