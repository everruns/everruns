/**
 * Decisions:
 * - Keep event-to-component routing in one place so the transcript stays easy to extend.
 * - Tool-only assistant events share the same grouping rules as explicit tool-call-requested events.
 * - Narrated act timelines suppress duplicate tool groups to avoid repeated progress chrome.
 * - Error message completions are canonical; matching turn failures only carry lifecycle state.
 */
"use client";

import { Bot, CalendarClock, Loader2, Sparkles, UserMinus, UserPlus } from "lucide-react";
import { Fragment, memo, useCallback, useMemo } from "react";
import type { ReactNode } from "react";
import type {
  ContentPart,
  Event,
  InputMessageData,
  OutputMessageCompletedData,
  SessionParticipant,
  TurnFailedData,
  ToolCompletedData,
  ToolProgressData,
} from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { getSessionParticipantLabel } from "@/lib/session-participant-label";
import type { ToolOutputStreams } from "@/app/(main)/sessions/[sessionId]/session-context";
import { getEventData, isImageFilePart, isTextPart } from "@/lib/api/types";
import type { TextAnnotation } from "@/lib/api/types";
import { useAgents, useProviders } from "@/hooks";
import { buildTraceConfigByDriver, resolveGenerationTraceUrl } from "@/lib/chat-trace";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { TraceLink } from "@/components/chat/trace-link";
import { MessageImage } from "@/components/chat/image-attachments";
import { MessageContent } from "@/components/chat/message-content";
import { WorkLogNarration } from "@/components/chat/work-log-narration";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import {
  getReasoningMultiIterationTurnIds,
  getKnownTurnId,
  isStructuralWorkLogEvent,
  shouldRenderWorkLogEvent,
} from "@/components/chat/chat-work-log-events";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { SetupConnectionToolCall } from "@/components/chat/setup-connection-tool-call";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";
import { buildToolActivityGroups } from "@/components/chat/tool-activity-groups";
import {
  formatWorkedDuration,
  getCompletedTurnIterationsByTurn,
  getCompletedTurnDurationsByEvent,
  getCompletedTurnDurationsByTurn,
} from "@/components/chat/turn-delimiter";
import { TurnWorkLog } from "@/components/chat/turn-work-log";
import { RunCards } from "@/components/chat/run-card";
import type { ChatRun } from "@/components/chat/run-cards";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { CompactionDivider } from "@/components/chat/compaction-divider";
import { ModelChangeDivider } from "@/components/chat/model-change-divider";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";
import { useLocale } from "@/providers/locale-provider";
import {
  getRuntimeErrorFromOutputMessage,
  getRuntimeErrorFromTurnFailed,
  localizeRuntimeError,
} from "@/lib/runtime-errors";
import type { SupportedLocale } from "@/lib/i18n";

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
  /**
   * Session participants, when known. Used to derive centered join/leave "system
   * lines" in the transcript (there is no participant SSE event). Optional and
   * guarded so single-host sessions render exactly as before.
   */
  participants?: SessionParticipant[];
  /**
   * Runs a turn started, keyed by the transcript row the turn ends on (see
   * `run-cards.ts`). Only the Chats thread surface passes this; session detail
   * keeps its transcript free of run chrome.
   */
  runsByEventId?: Map<string, ChatRun[]>;
}

/** A derived join/leave marker interleaved into the transcript by timestamp. */
interface ParticipantMarker {
  id: string;
  ts: string;
  kind: "join" | "leave";
  participant: SessionParticipant;
}

function ReasoningLogRow({ text }: { text: string }) {
  return (
    <div className="flex items-start gap-2 py-1 text-[15px] leading-6 text-muted-foreground">
      <Sparkles className="mt-1 h-3.5 w-3.5 flex-shrink-0 text-primary/70" />
      <WorkLogNarration>{text}</WorkLogNarration>
    </div>
  );
}

function getMessageImages(content: ContentPart[]): Array<{ image_id: string; filename?: string }> {
  return content.filter(isImageFilePart).map((part) => ({
    image_id: part.image_id,
    filename: part.filename,
  }));
}

function getMessageAnnotations(content: ContentPart[] | undefined): TextAnnotation[] {
  if (!content) return [];
  return content.flatMap((part) => (isTextPart(part) && part.annotations ? part.annotations : []));
}

function getTurnFailedMessage(locale: SupportedLocale, data: TurnFailedData): string {
  return localizeRuntimeError(locale, getRuntimeErrorFromTurnFailed(data), "");
}

function renderTurnDivider(
  eventId: string,
  turnDurationByEventId: Map<string, number>,
  workedForText: string | null,
  traceUrl: string | null,
  traceLabel: string,
) {
  const durationMs = turnDurationByEventId.get(eventId);
  if (durationMs == null || !workedForText) return null;

  return (
    <div className="flex items-center gap-4 pt-3 text-xs font-medium text-muted-foreground sm:text-sm">
      <div className="h-px flex-1 bg-border" />
      <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
        {workedForText}
        {traceUrl && <TraceLink href={traceUrl} label={traceLabel} />}
      </span>
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
  participants,
  runsByEventId,
}: ChatMessageListProps) {
  const { locale, t } = useLocale();
  const { data: providers } = useProviders();
  const { data: agents } = useAgents();
  const traceConfigByDriver = useMemo(() => buildTraceConfigByDriver(providers), [providers]);
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
  const errorMessageTurnIds = useMemo(() => {
    const ids = new Set<string>();
    for (const event of chatEvents) {
      const data = getEventData(event, "output.message.completed");
      if (!getRuntimeErrorFromOutputMessage(data)) continue;
      const turnId = getKnownTurnId(event);
      if (turnId) ids.add(turnId);
    }
    return ids;
  }, [chatEvents]);

  const turnDurationByEventId = useMemo(
    () => getCompletedTurnDurationsByEvent(events ?? []),
    [events],
  );
  const turnDurationByTurnId = useMemo(
    () => getCompletedTurnDurationsByTurn(events ?? []),
    [events],
  );
  const turnIterationsByTurnId = useMemo(
    () => getCompletedTurnIterationsByTurn(events ?? []),
    [events],
  );
  const structuralWorkTurnIds = useMemo(() => {
    const ids = new Set<string>();
    for (const event of chatEvents) {
      if (!isStructuralWorkLogEvent(event)) continue;
      const turnId = getKnownTurnId(event);
      if (turnId) ids.add(turnId);
    }
    return ids;
  }, [chatEvents]);
  const reasoningMultiIterationTurnIds = useMemo(
    () => getReasoningMultiIterationTurnIds(chatEvents),
    [chatEvents],
  );
  const isWorkLogEvent = useCallback(
    (event: Event) => {
      return shouldRenderWorkLogEvent(
        event,
        turnIterationsByTurnId,
        structuralWorkTurnIds,
        reasoningMultiIterationTurnIds,
      );
    },
    [reasoningMultiIterationTurnIds, structuralWorkTurnIds, turnIterationsByTurnId],
  );
  const workLogTurnIds = useMemo(() => {
    const ids = new Set<string>();
    for (const event of chatEvents) {
      if (!isWorkLogEvent(event)) continue;
      const turnId = getKnownTurnId(event);
      if (turnId) ids.add(turnId);
    }
    return ids;
  }, [chatEvents, isWorkLogEvent]);
  const workLogEventsByTurnId = useMemo(() => {
    const groups = new Map<string, Event[]>();
    for (const event of chatEvents) {
      if (!isWorkLogEvent(event)) continue;
      const turnId = getKnownTurnId(event);
      if (!turnId) continue;
      const group = groups.get(turnId);
      if (group) {
        group.push(event);
      } else {
        groups.set(turnId, [event]);
      }
    }
    return groups;
  }, [chatEvents, isWorkLogEvent]);
  const activityGroups = useMemo(
    () => buildToolActivityGroups(chatEvents, t("working")),
    [chatEvents, t],
  );

  // Agent display-name lookup for participant system lines.
  const agentNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const agent of agents ?? []) {
      map.set(agent.id, getDisplayName(agent));
    }
    return map;
  }, [agents]);

  // Derive join/leave markers from participants. There is no participant SSE
  // event, so lines are inferred from `joined_at` / `left_at`. Guarded: only a
  // multi-participant session produces markers, and the original host's join is
  // suppressed so ordinary 1:1 transcripts stay clean.
  const participantMarkers = useMemo<ParticipantMarker[]>(() => {
    if (!participants || participants.length < 2) return [];
    const markers: ParticipantMarker[] = [];
    for (const p of participants) {
      if (p.role !== "host") {
        markers.push({ id: `join-${p.id}`, ts: p.joined_at, kind: "join", participant: p });
      }
      if (p.left_at) {
        markers.push({ id: `leave-${p.id}`, ts: p.left_at, kind: "leave", participant: p });
      }
    }
    markers.sort((a, b) => a.ts.localeCompare(b.ts));
    return markers;
  }, [participants]);

  // Assign each marker to the first transcript event whose timestamp is at or
  // after it; markers after the last event render as a trailing block.
  const { markersByEventId, trailingMarkers } = useMemo(() => {
    const byEvent = new Map<string, ParticipantMarker[]>();
    const trailing: ParticipantMarker[] = [];
    if (participantMarkers.length === 0) {
      return { markersByEventId: byEvent, trailingMarkers: trailing };
    }
    let mi = 0;
    for (const event of chatEvents) {
      while (mi < participantMarkers.length && participantMarkers[mi].ts <= event.ts) {
        const list = byEvent.get(event.id) ?? [];
        list.push(participantMarkers[mi]);
        byEvent.set(event.id, list);
        mi += 1;
      }
    }
    while (mi < participantMarkers.length) {
      trailing.push(participantMarkers[mi]);
      mi += 1;
    }
    return { markersByEventId: byEvent, trailingMarkers: trailing };
  }, [participantMarkers, chatEvents]);

  const participantLabel = useCallback(
    (p: SessionParticipant): string => getSessionParticipantLabel(p, agentNameById),
    [agentNameById],
  );

  const renderParticipantMarker = useCallback(
    (marker: ParticipantMarker): ReactNode => {
      const name = participantLabel(marker.participant);
      const isJoin = marker.kind === "join";
      return (
        <div
          key={marker.id}
          className="flex items-center gap-4 py-2 text-xs font-medium text-muted-foreground sm:text-sm"
        >
          <div className="h-px flex-1 bg-border" />
          <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
            {isJoin ? <UserPlus className="h-3.5 w-3.5" /> : <UserMinus className="h-3.5 w-3.5" />}
            {isJoin ? `${name} joined the session` : `${name} left the session`}
          </span>
          <div className="h-px flex-1 bg-border" />
        </div>
      );
    },
    [participantLabel],
  );

  const renderWorkLog = (event: Event, children: ReactNode) => {
    const turnId = getKnownTurnId(event);
    const durationMs = turnId ? turnDurationByTurnId.get(turnId) : undefined;
    const label =
      durationMs == null
        ? t("working")
        : t("worked_for", { duration: formatWorkedDuration(durationMs) });

    return (
      <TurnWorkLog key={event.id} label={label} isActive={durationMs == null}>
        {children}
      </TurnWorkLog>
    );
  };
  const renderWorkLogEventContent = (event: Event) => {
    const reasonItemData = getEventData(event, "reason.item");
    if (reasonItemData) {
      const summary = (reasonItemData.summary ?? [])
        .map((item) => item.trim())
        .filter(Boolean)
        .join("\n");
      return summary ? <ReasoningLogRow key={event.id} text={summary} /> : null;
    }

    const reasonCompletedData = getEventData(event, "reason.completed");
    if (reasonCompletedData) {
      return reasonCompletedData.text_preview ? (
        <ReasoningLogRow key={event.id} text={reasonCompletedData.text_preview} />
      ) : null;
    }

    const group = activityGroups.byAnchorEventId.get(event.id);
    if (group) {
      const requested = getEventData(event, "tool.call_requested");
      const connectionCalls =
        requested?.tool_calls.filter((toolCall) => toolCall.name === "setup_connection") ?? [];
      return (
        <div key={event.id} className="space-y-1">
          <ToolActivityTimelineGroup
            headline={group.headline}
            completedHeadline={group.completedHeadline}
            rows={group.rows}
          />
          {connectionCalls.map((toolCall) => (
            <SetupConnectionToolCall
              key={toolCall.id}
              sessionId={sessionId}
              toolCallId={toolCall.id}
              provider={(toolCall.arguments as { provider?: string })?.provider ?? "unknown"}
              toolResultsMap={toolResultsMap}
            />
          ))}
        </div>
      );
    }

    if (activityGroups.groupedEventIds.has(event.id)) return null;

    if (event.type !== "tool.call_requested") return null;

    const reqData = getEventData(event, "tool.call_requested");
    if (!reqData?.tool_calls?.length) return null;

    const connectionCalls = reqData.tool_calls.filter(
      (toolCall) => toolCall.name === "setup_connection",
    );

    return (
      <div key={event.id} className="space-y-1">
        {connectionCalls.map((toolCall) => (
          <SetupConnectionToolCall
            key={toolCall.id}
            sessionId={sessionId}
            toolCallId={toolCall.id}
            provider={(toolCall.arguments as { provider?: string })?.provider ?? "unknown"}
            toolResultsMap={toolResultsMap}
          />
        ))}
      </div>
    );
  };

  if (eventsLoading) {
    return (
      <div className="space-y-4">
        <div className="ml-auto h-20 w-3/4 animate-pulse bg-muted" />
        <div className="h-20 w-3/4 animate-pulse bg-muted" />
        <div className="h-20 w-2/3 animate-pulse bg-muted" />
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
    <div className="space-y-4">
      {loadingOlderEvents && hasMoreEvents && (
        <div className="flex items-center justify-center py-2.5">
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          <span className="ml-2 text-xs text-muted-foreground">{t("loading_older_messages")}</span>
        </div>
      )}
      {chatEvents.map((event) => {
        const eventNode = ((): ReactNode => {
          if (event.type === "context.compacted") {
            const compactedData = getEventData(event, "context.compacted");
            return compactedData ? <CompactionDivider key={event.id} data={compactedData} /> : null;
          }

          if (event.type === "session.model.changed") {
            const modelChangedData = getEventData(event, "session.model.changed");
            return modelChangedData ? (
              <ModelChangeDivider key={event.id} data={modelChangedData} />
            ) : null;
          }

          const turnFailedData = getEventData(event, "turn.failed");
          if (turnFailedData) {
            if (errorMessageTurnIds.has(turnFailedData.turn_id)) return null;
            return (
              <ChatErrorAlert
                key={event.id}
                message={getTurnFailedMessage(locale, turnFailedData)}
              />
            );
          }

          if (isWorkLogEvent(event)) {
            const turnId = getKnownTurnId(event);
            if (turnId) {
              const group = workLogEventsByTurnId.get(turnId) ?? [];
              if (group[0]?.id !== event.id) return null;
              return renderWorkLog(
                event,
                <div className="space-y-3">
                  {group.map((groupEvent) => renderWorkLogEventContent(groupEvent))}
                </div>,
              );
            }

            return renderWorkLog(event, renderWorkLogEventContent(event));
          }

          const isUser = event.type === "input.message";
          const inputData = getEventData(event, "input.message");
          const outputData = getEventData(event, "output.message.completed");
          const data = inputData ?? outputData;
          if (!data) return null;

          const textContent = getMessageText(data);
          const outputError = outputData ? getRuntimeErrorFromOutputMessage(outputData) : undefined;
          if (outputData && outputError) {
            return <ChatErrorAlert key={event.id} message={textContent} />;
          }
          const outputToolCalls = !isUser && outputData ? getToolCalls(outputData) : [];
          const toolCalls = outputToolCalls.filter(
            (toolCall) =>
              !clientRequestedToolCallIds.has(toolCall.id) &&
              !activityGroups.narratedToolCallIds.has(toolCall.id),
          );
          const images = data.message?.content ? getMessageImages(data.message.content) : [];
          const annotations = isUser ? [] : getMessageAnnotations(data.message?.content);
          // Deep link to this generation's trace on the provider (assistant
          // messages only; user messages carry no provider response id).
          const genTraceUrl = outputData
            ? resolveGenerationTraceUrl(outputData.message?.metadata, traceConfigByDriver, {
                sessionId,
                turnId: event.context?.turn_id,
              })
            : null;
          const isScheduleTriggered = isUser && data.message?.metadata?.source === "schedule";
          const isToolOnlyMessage =
            !isUser && outputToolCalls.length > 0 && !textContent && images.length === 0;

          if (isToolOnlyMessage) {
            if (toolCalls.length === 0) return null;
            return renderWorkLog(
              event,
              <div className="space-y-1">
                <ToolActivityGroup
                  toolCalls={toolCalls}
                  toolResultsMap={toolResultsMap}
                  toolProgressMap={toolProgressMap}
                  toolOutputMap={toolOutputMap}
                />
              </div>,
            );
          }

          return (
            <div
              key={event.id}
              className={`chat-transcript-row space-y-2 ${isUser ? "scroll-mt-4" : ""}`}
              data-message-anchor={isUser ? event.id : undefined}
            >
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
                          {textContent && (
                            <MessageContent text={textContent} annotations={annotations} />
                          )}
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
                        {genTraceUrl && (
                          <TraceLink href={genTraceUrl} label={t("trace_view_message")} />
                        )}
                      </div>
                    </div>
                  )}
                </div>
              )}

              {toolCalls.length > 0 && (
                <div className="ml-9 space-y-1">
                  <ToolActivityGroup
                    toolCalls={toolCalls}
                    toolResultsMap={toolResultsMap}
                    toolProgressMap={toolProgressMap}
                    toolOutputMap={toolOutputMap}
                  />
                </div>
              )}

              {(() => {
                const runs = runsByEventId?.get(event.id);
                return runs ? <RunCards runs={runs} /> : null;
              })()}

              {(() => {
                const turnId = getKnownTurnId(event);
                if (turnId && workLogTurnIds.has(turnId)) return null;
                return renderTurnDivider(
                  event.id,
                  turnDurationByEventId,
                  t("worked_for", {
                    duration: formatWorkedDuration(turnDurationByEventId.get(event.id) ?? 0),
                  }),
                  genTraceUrl,
                  t("trace_view_turn"),
                );
              })()}
            </div>
          );
        })();

        const participantLines = markersByEventId.get(event.id);
        if (participantLines && participantLines.length > 0) {
          return (
            <Fragment key={`row-${event.id}`}>
              {participantLines.map((marker) => renderParticipantMarker(marker))}
              {eventNode}
            </Fragment>
          );
        }
        return eventNode;
      })}
      {trailingMarkers.map((marker) => renderParticipantMarker(marker))}
    </div>
  );
});
