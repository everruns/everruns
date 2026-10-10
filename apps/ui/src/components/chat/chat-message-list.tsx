/**
 * Decisions:
 * - Keep event-to-component routing in one place so the transcript stays easy to extend.
 * - Tool-only assistant events share the same grouping rules as explicit tool-call-requested events.
 * - Narrated act timelines suppress duplicate tool groups to avoid repeated progress chrome.
 * - Error message completions are canonical; matching turn failures only carry lifecycle state.
 * - Commentary and live thinking sit with tool calls inside the work log. Final answers stay messages.
 * - `conversation.message` (sent with `send_message` under explicit communication) is the agent's
 *   reply and renders as an assistant bubble; its assistant text is commentary in the work log.
 * - In folded chat, each user message owns its turn status row from Enter to the answer
 *   (knowledge/ui/chat-experience.md). Rows sit in one flat, keyed list with the pending sends
 *   after the stored events, so a row keyed by the client id survives the moment its send is
 *   stored. A turn whose user message is out of view keeps the older placement: its work log
 *   at its first work event.
 */
"use client";

import { AgentIcon } from "@/components/icons/facet-icons";
import {
  CalendarClock,
  Loader2,
  MessageSquare,
  RefreshCw,
  Sparkles,
  UserMinus,
  UserPlus,
} from "lucide-react";
import { Fragment, memo, useCallback, useEffect, useMemo, useState } from "react";
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
import { parseCoordinatorMessage, parseTaskUpdate } from "@/lib/chat-thread-messages";
import { TraceLink } from "@/components/chat/trace-link";
import { MessageImage } from "@/components/chat/image-attachments";
import { MessageContent } from "@/components/chat/message-content";
import { WorkLogNarration } from "@/components/chat/work-log-narration";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import {
  getReasoningMultiIterationTurnIds,
  hasReasoningWorkLogSummary,
  getKnownTurnId,
  isCommentaryWorkLogEvent,
  isStructuralWorkLogEvent,
  shouldRenderWorkLogEvent,
} from "@/components/chat/chat-work-log-events";
import { ThinkingIndicator } from "@/components/thinking-indicator";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { SetupConnectionToolCall } from "@/components/chat/setup-connection-tool-call";
import {
  UrlElicitationToolCall,
  type UrlElicitationArguments,
} from "@/components/chat/url-elicitation-tool-call";
import {
  MCP_APPROVAL_TOOL,
  McpApprovalToolCall,
  type McpApprovalArguments,
} from "@/components/chat/mcp-approval-tool-call";
import {
  TOOL_APPROVAL_TOOL,
  ToolApprovalRequests,
} from "@/components/chat/tool-approval-tool-call";
import { AskUserToolCall, isAskUserArguments } from "@/components/chat/ask-user-tool-call";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";
import { buildToolActivityGroups } from "@/components/chat/tool-activity-groups";
import {
  formatWorkedDuration,
  getCompletedTurnIterationsByTurn,
  getCompletedTurnDurationsByEvent,
  getCompletedTurnDurationsByTurn,
} from "@/components/chat/turn-delimiter";
import { TurnWorkLog, WorkLogEntries } from "@/components/chat/turn-work-log";
import { RunCards } from "@/components/chat/run-card";
import type { ChatRun } from "@/components/chat/run-cards";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { CompactionDivider } from "@/components/chat/compaction-divider";
import { ModelChangeDivider } from "@/components/chat/model-change-divider";
import { getFullText, type ToolCallContent } from "@/components/chat/tool-call-utils";
import type { ApprovalToolContext } from "@/components/chat/approval-tool-activity";
import { useMembers } from "@/hooks/use-members";
import { useLocale } from "@/providers/locale-provider";
import {
  getRuntimeErrorFromOutputMessage,
  getRuntimeErrorFromTurnFailed,
  localizeRuntimeError,
} from "@/lib/runtime-errors";
import type { SupportedLocale } from "@/lib/i18n";
import {
  buildTurnIndex,
  getClientMessageId,
  isLiveTurnPhase,
  rowForPendingSend,
  rowForStoredMessage,
  type PendingSend,
  type TurnRowState,
} from "@/lib/chat-turn-state";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { cn } from "@/lib/utils";
import { formatDaySeparator, needsDaySeparator } from "@/lib/chat-day-separator";

/** A stored message this recent may still be waiting for its turn to start. */
const STARTING_GRACE_MS = 60_000;

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
  /**
   * Replaces the default "No messages yet" card when the transcript is empty.
   * Used when the surface has something more useful to say than "start typing"
   * — e.g. the org has no model to chat with, so inviting a first message would
   * be a second, competing centred message.
   */
  emptyState?: ReactNode;
  /** Fold turn activity for human chat; testing/debugging surfaces show it inline. */
  collapseWorkLog?: boolean;
  /**
   * Live thinking or commentary for the active turn. Rendered with tool calls
   * inside the work log instead of as an assistant message.
   */
  streamingWork?: {
    turnId: string | null;
    text: string | null;
    isThinking: boolean;
  } | null;
  /** Sends the event stream does not carry yet; each gets a user block and status row. */
  pendingSends?: PendingSend[];
  onRetrySend?: (clientId: string) => void;
  /** The session reports a running turn. */
  sessionActive?: boolean;
}

interface SetupConnectionArguments {
  provider?: string;
  subject?: "agent" | "user";
  setup_url?: string;
}

/** A derived join/leave marker interleaved into the transcript by timestamp. */
interface ParticipantMarker {
  id: string;
  ts: string;
  kind: "join" | "leave";
  participant: SessionParticipant;
}

function StreamingWorkRow({ text, isThinking }: { text: string | null; isThinking: boolean }) {
  if (text) return <ReasoningLogRow text={text} />;
  if (isThinking) return <ThinkingIndicator />;
  return null;
}

function ReasoningLogRow({ text }: { text: string }) {
  return (
    <div className="flex items-start gap-2 py-1 text-[15px] leading-6 text-muted-foreground">
      <Sparkles className="mt-1 h-3.5 w-3.5 flex-shrink-0 text-primary/70" />
      <WorkLogNarration>{text}</WorkLogNarration>
    </div>
  );
}

/** First non-empty line of a Markdown summary, with inline emphasis stripped. */
function getFirstPlainLine(text: string): string {
  const line = text
    .split("\n")
    .map((part) =>
      part
        .replace(/^#+\s*/, "")
        .replace(/[*_`]/g, "")
        .trim(),
    )
    .find(Boolean);
  return line ?? "";
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

const EMPTY_PENDING_SENDS: PendingSend[] = [];

/** Ticks once a second while `active`, for rows that change with time alone. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
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
  emptyState,
  collapseWorkLog = true,
  streamingWork = null,
  pendingSends = EMPTY_PENDING_SENDS,
  onRetrySend,
  sessionActive = false,
}: ChatMessageListProps) {
  const { locale, t } = useLocale();
  const { data: providers } = useProviders();
  const { data: agents } = useAgents();
  const approvalCalls = useMemo(() => {
    const calls: ToolCallContent[] = [];
    for (const event of chatEvents) {
      const output = getEventData(event, "output.message.completed");
      if (!output) continue;
      calls.push(...getToolCalls(output).filter((call) => call.name === "record_approval"));
    }
    return calls;
  }, [chatEvents, getToolCalls]);
  const { data: members } = useMembers(approvalCalls.length > 0);
  const memberNames = useMemo(
    () => new Map((members ?? []).map((member) => [member.user_id, member.name || member.email])),
    [members],
  );
  const approvalContexts = useMemo(() => {
    const inputEventsByMessageId = new Map<string, Event>();
    for (const event of chatEvents) {
      const input = getEventData(event, "input.message");
      if (input?.message.id) inputEventsByMessageId.set(input.message.id, event);
    }

    const contexts = new Map<string, ApprovalToolContext>();
    for (const call of approvalCalls) {
      const resultText = getFullText(toolResultsMap.get(call.id)?.result);
      let approvedMessageId: string | undefined;
      try {
        const payload: unknown = JSON.parse(resultText);
        if (payload && typeof payload === "object" && "approved_in_message" in payload) {
          const value = payload.approved_in_message;
          if (typeof value === "string") approvedMessageId = value;
        }
      } catch {
        // Missing correlation leaves the consent link unavailable.
      }

      const inputEvent = approvedMessageId
        ? inputEventsByMessageId.get(approvedMessageId)
        : undefined;
      const input = inputEvent ? getEventData(inputEvent, "input.message") : undefined;
      const metadata = inputEvent?.metadata ?? input?.message.metadata;
      const initiator = metadata?.initiator;
      const actorId =
        initiator &&
        typeof initiator === "object" &&
        "type" in initiator &&
        initiator.type === "user" &&
        "user_id" in initiator &&
        typeof initiator.user_id === "string"
          ? initiator.user_id
          : undefined;
      contexts.set(call.id, {
        approvedBy: actorId
          ? memberNames.get(actorId) || `user ${actorId}`
          : inputEvent
            ? "an unattributed initiator"
            : "an unknown actor",
        consentMessageHref: approvedMessageId
          ? `/sessions/${sessionId}/chat#message-${approvedMessageId}`
          : undefined,
      });
    }
    return contexts;
  }, [approvalCalls, chatEvents, memberNames, sessionId, toolResultsMap]);
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
      if (isCommentaryWorkLogEvent(event)) return true;
      if (!collapseWorkLog) {
        return isStructuralWorkLogEvent(event) || hasReasoningWorkLogSummary(event);
      }
      return shouldRenderWorkLogEvent(
        event,
        turnIterationsByTurnId,
        structuralWorkTurnIds,
        reasoningMultiIterationTurnIds,
      );
    },
    [
      collapseWorkLog,
      reasoningMultiIterationTurnIds,
      structuralWorkTurnIds,
      turnIterationsByTurnId,
    ],
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
  const turnStartedAtByTurnId = useMemo(() => {
    const startedAt = new Map<string, string>();
    for (const event of events ?? []) {
      const data = getEventData(event, "turn.started");
      if (data && !startedAt.has(data.turn_id)) startedAt.set(data.turn_id, event.ts);
    }
    return startedAt;
  }, [events]);
  const activityGroups = useMemo(
    () => buildToolActivityGroups(chatEvents, t("working"), locale),
    [chatEvents, t, locale],
  );

  // The newest reply rendered as prose keeps its actions visible.
  const latestAgentEventId = useMemo(() => {
    for (let index = chatEvents.length - 1; index >= 0; index -= 1) {
      const event = chatEvents[index];
      const output = getEventData(event, "output.message.completed");
      if (!output || isWorkLogEvent(event) || getRuntimeErrorFromOutputMessage(output)) continue;
      if (getMessageText(output).trim()) return event.id;
    }
    return undefined;
  }, [chatEvents, getMessageText, isWorkLogEvent]);

  // Turn status rows (folded chat only).
  const turnIndex = useMemo(() => buildTurnIndex(events ?? []), [events]);
  const { anchoredTurnIds, latestUserEventId, lastIdledSequence } = useMemo(() => {
    const anchored = new Set<string>();
    let latest: string | undefined;
    let lastIdled = -1;
    if (collapseWorkLog) {
      for (const event of chatEvents) {
        const input = getEventData(event, "input.message");
        if (!input) continue;
        latest = event.id;
        const turnId = turnIndex.turnByInputMessageId.get(input.message.id);
        if (turnId) anchored.add(turnId);
      }
      for (const event of events ?? []) {
        if (event.type === "session.idled" && typeof event.sequence === "number") {
          lastIdled = Math.max(lastIdled, event.sequence);
        }
      }
    }
    return { anchoredTurnIds: anchored, latestUserEventId: latest, lastIdledSequence: lastIdled };
  }, [chatEvents, collapseWorkLog, events, turnIndex]);
  const hasSendingSend = pendingSends.some((send) => send.phase === "sending");
  const now = useNow(hasSendingSend);

  // A retried reason activity re-emits the turn's model-change marker, and the
  // same switch twice in a row is noise, not history. Keep the first of each
  // consecutive run; a genuine switch back (A→B→A) still renders every step.
  const repeatedModelChangeIds = useMemo(() => {
    const repeated = new Set<string>();
    let previous: string | undefined;
    for (const event of chatEvents) {
      const data = getEventData(event, "session.model.changed");
      if (!data) continue;
      const change = `${data.previous_model_id ?? ""}→${data.model_id}`;
      if (change === previous) repeated.add(event.id);
      previous = change;
    }
    return repeated;
  }, [chatEvents]);

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

  // Cards that ask the user for something (connection setup, URL elicitation,
  // approvals, ask_user). While the turn runs they render outside the folded
  // work log so a pending request is never hidden behind the collapse.
  const renderInteractiveToolCalls = (event: Event): ReactNode => {
    const requested = getEventData(event, "tool.call_requested");
    if (!requested?.tool_calls?.length) return null;

    const connectionCalls = requested.tool_calls.filter(
      (toolCall) => toolCall.name === "setup_connection",
    );
    const elicitationCalls = requested.tool_calls.filter(
      (toolCall) => toolCall.name === "confirm_url_elicitation",
    );
    const askUserCalls = requested.tool_calls.filter((toolCall) => toolCall.name === "ask_user");
    const approvalCalls = requested.tool_calls.filter(
      (toolCall) => toolCall.name === MCP_APPROVAL_TOOL,
    );
    const toolApprovalCalls = requested.tool_calls.filter(
      (toolCall) => toolCall.name === TOOL_APPROVAL_TOOL,
    );
    if (
      connectionCalls.length +
        elicitationCalls.length +
        askUserCalls.length +
        approvalCalls.length +
        toolApprovalCalls.length ===
      0
    ) {
      return null;
    }

    return (
      <Fragment key={`interactive-${event.id}`}>
        {connectionCalls.map((toolCall) => (
          <SetupConnectionToolCall
            key={toolCall.id}
            sessionId={sessionId}
            toolCallId={toolCall.id}
            provider={(toolCall.arguments as SetupConnectionArguments)?.provider ?? "unknown"}
            subject={(toolCall.arguments as SetupConnectionArguments)?.subject}
            setupUrl={(toolCall.arguments as SetupConnectionArguments)?.setup_url}
            toolResultsMap={toolResultsMap}
          />
        ))}
        {elicitationCalls.map((toolCall) => (
          <UrlElicitationToolCall
            key={toolCall.id}
            sessionId={sessionId}
            toolCallId={toolCall.id}
            elicitation={(toolCall.arguments ?? {}) as UrlElicitationArguments}
            toolResultsMap={toolResultsMap}
          />
        ))}
        {approvalCalls.map((toolCall) => (
          <McpApprovalToolCall
            key={toolCall.id}
            sessionId={sessionId}
            toolCallId={toolCall.id}
            approval={(toolCall.arguments ?? {}) as McpApprovalArguments}
            toolResultsMap={toolResultsMap}
          />
        ))}
        {toolApprovalCalls.length > 0 && (
          <ToolApprovalRequests
            sessionId={sessionId}
            requests={toolApprovalCalls}
            toolResultsMap={toolResultsMap}
          />
        )}
        {askUserCalls.map((toolCall) =>
          isAskUserArguments(toolCall.arguments) ? (
            <AskUserToolCall
              key={toolCall.id}
              sessionId={sessionId}
              toolCallId={toolCall.id}
              request={toolCall.arguments}
              requestedAt={event.ts}
              toolResultsMap={toolResultsMap}
            />
          ) : null,
        )}
      </Fragment>
    );
  };

  // One line for the collapsed header: the newest tool group's headline, or the
  // first line of the newest reasoning summary.
  const getWorkLogStatus = (workEvents: Event[]): string | undefined => {
    for (let index = workEvents.length - 1; index >= 0; index -= 1) {
      const event = workEvents[index];
      const group = activityGroups.byAnchorEventId.get(event.id);
      if (group) {
        const running = group.rows.some(
          (row) => row.state === "running" || row.state === "waiting",
        );
        return running ? group.headline : (group.completedHeadline ?? group.headline);
      }
      const summary = getEventData(event, "reason.item")?.summary?.join("\n");
      const commentary = getEventData(event, "output.message.completed");
      const commentaryText =
        commentary && isCommentaryWorkLogEvent(event) ? getMessageText(commentary) : "";
      const line = getFirstPlainLine(summary || commentaryText);
      if (line) return line;
    }
    return undefined;
  };

  const countWorkLogErrors = (workEvents: Event[]): number =>
    workEvents.reduce(
      (total, event) =>
        total +
        (activityGroups.byAnchorEventId.get(event.id)?.rows.filter((row) => row.state === "error")
          .length ?? 0),
      0,
    );

  const renderStreamingWorkRow = () =>
    streamingWork ? (
      <StreamingWorkRow
        key="streaming-work"
        text={streamingWork.text}
        isThinking={streamingWork.isThinking}
      />
    ) : null;

  const liveWorkStatus = (turnId: string | undefined): string | undefined => {
    if (!streamingWork?.text || !turnId || streamingWork.turnId !== turnId) return undefined;
    return getFirstPlainLine(streamingWork.text) || undefined;
  };

  const renderWorkLog = (
    event: Event,
    workEvents: Event[],
    renderBody: (isActive: boolean) => ReactNode,
    extraErrorCount = 0,
  ) => {
    if (!collapseWorkLog) return <Fragment key={event.id}>{renderBody(false)}</Fragment>;
    const turnId = getKnownTurnId(event);
    const durationMs = turnId ? turnDurationByTurnId.get(turnId) : undefined;
    const isActive = durationMs == null;
    const label =
      durationMs == null
        ? t("working")
        : t("worked_for", { duration: formatWorkedDuration(durationMs) });
    const startedAt = (turnId && turnStartedAtByTurnId.get(turnId)) || event.ts;
    const startedAtMs = Date.parse(startedAt);
    const attentionCards = isActive
      ? workEvents.map((workEvent) => renderInteractiveToolCalls(workEvent)).filter(Boolean)
      : [];

    return (
      <TurnWorkLog
        key={event.id}
        label={label}
        isActive={isActive}
        startedAtMs={Number.isNaN(startedAtMs) ? undefined : startedAtMs}
        status={isActive ? liveWorkStatus(turnId) || getWorkLogStatus(workEvents) : undefined}
        errorCount={countWorkLogErrors(workEvents) + extraErrorCount}
        attention={attentionCards.length > 0 ? attentionCards : null}
      >
        {() => renderBody(isActive)}
      </TurnWorkLog>
    );
  };
  const renderWorkLogEventContent = (event: Event, includeInteractive: boolean) => {
    const commentary = getEventData(event, "output.message.completed");
    if (commentary && isCommentaryWorkLogEvent(event)) {
      const text = getMessageText(commentary).trim();
      const toolCalls = getToolCalls(commentary).filter(
        (toolCall) =>
          !clientRequestedToolCallIds.has(toolCall.id) &&
          !activityGroups.narratedToolCallIds.has(toolCall.id),
      );
      if (!text && toolCalls.length === 0) return null;
      return (
        <div key={event.id} className="space-y-1">
          {text ? <ReasoningLogRow text={text} /> : null}
          {toolCalls.length > 0 && (
            <ToolActivityGroup
              toolCalls={toolCalls}
              toolResultsMap={toolResultsMap}
              toolProgressMap={toolProgressMap}
              toolOutputMap={toolOutputMap}
              approvalContexts={approvalContexts}
            />
          )}
        </div>
      );
    }

    const reasonItemData = getEventData(event, "reason.item");
    if (reasonItemData) {
      const summary = (reasonItemData.summary ?? [])
        .map((item) => item.trim())
        .filter(Boolean)
        .join("\n");
      return summary ? <ReasoningLogRow key={event.id} text={summary} /> : null;
    }

    const interactive = includeInteractive ? renderInteractiveToolCalls(event) : null;
    const group = activityGroups.byAnchorEventId.get(event.id);
    if (group) {
      return (
        <div key={event.id} className="space-y-1">
          <ToolActivityTimelineGroup
            headline={group.headline}
            completedHeadline={group.completedHeadline}
            rows={group.rows}
            collapsible={collapseWorkLog}
          />
          {interactive}
        </div>
      );
    }

    if (activityGroups.groupedEventIds.has(event.id)) return null;
    if (!interactive) return null;

    return (
      <div key={event.id} className="space-y-1">
        {interactive}
      </div>
    );
  };

  const streamingTurnHasLog =
    !!streamingWork?.turnId &&
    (workLogTurnIds.has(streamingWork.turnId) || anchoredTurnIds.has(streamingWork.turnId));
  const streamingRow = streamingWork ? renderStreamingWorkRow() : null;
  const trailingStreamingWork =
    streamingRow && (!collapseWorkLog || !streamingTurnHasLog) ? (
      collapseWorkLog ? (
        <TurnWorkLog
          label={t("working")}
          isActive
          status={
            streamingWork?.text ? getFirstPlainLine(streamingWork.text) || undefined : undefined
          }
        >
          {() => streamingRow}
        </TurnWorkLog>
      ) : (
        streamingRow
      )
    ) : null;

  const renderTurnRow = (key: string, row: TurnRowState) => {
    const turnId = row.turnId;
    const group = turnId ? (workLogEventsByTurnId.get(turnId) ?? []) : [];
    const live = isLiveTurnPhase(row.phase);
    const streamsHere =
      !!turnId && !!streamingRow && !!streamingWork?.text && streamingWork.turnId === turnId;
    const attention = live
      ? group.map((workEvent) => renderInteractiveToolCalls(workEvent)).filter(Boolean)
      : [];
    return (
      <TurnWorkLog
        key={key}
        phase={row.phase}
        startedAtMs={row.startedAtMs}
        durationMs={row.durationMs}
        hasSteps={group.length > 0 || streamsHere}
        status={live ? liveWorkStatus(turnId) || getWorkLogStatus(group) : undefined}
        errorCount={countWorkLogErrors(group)}
        attention={attention.length > 0 ? attention : null}
      >
        {() => {
          const entries = group
            .map((groupEvent) => renderWorkLogEventContent(groupEvent, !live))
            .filter(Boolean);
          if (streamsHere) entries.push(streamingRow);
          return <WorkLogEntries entries={entries} />;
        }}
      </TurnWorkLog>
    );
  };

  const renderStoredTurnRow = (event: Event): ReactNode => {
    if (!collapseWorkLog) return null;
    const input = getEventData(event, "input.message");
    if (!input) return null;
    // Optimistic and fixture events can lack a sequence; a turn that names the
    // message still anchors, and the steered-turn fallback just never matches.
    const sequence =
      typeof event.sequence === "number" && event.sequence >= 0
        ? event.sequence
        : Number.MAX_SAFE_INTEGER;
    const storedAtMs = Date.parse(event.ts);
    const row = rowForStoredMessage(turnIndex, {
      messageId: input.message.id,
      sequence,
      storedAtMs,
      isLatest: event.id === latestUserEventId,
      sessionActive:
        sessionActive ||
        (sequence > lastIdledSequence && Date.now() - storedAtMs < STARTING_GRACE_MS),
      hasSteps: (turnId) =>
        (workLogEventsByTurnId.get(turnId)?.length ?? 0) > 0 ||
        (!!streamingWork?.text && streamingWork.turnId === turnId),
    });
    if (!row) return null;
    return renderTurnRow(`turn-row-${getClientMessageId(event) ?? input.message.id}`, row);
  };

  const renderPendingSend = (send: PendingSend): ReactNode[] => {
    const images = send.images.map((image) => ({
      image_id: image.imageId,
      filename: image.filename,
    }));
    const nodes: ReactNode[] = [
      <div
        key={`user-${send.clientId}`}
        className="chat-transcript-row scroll-mt-4 space-y-2"
        data-pending-send={send.phase}
      >
        <div className="flex justify-end">
          <div className={chatSurfaceStyles.userMessage}>
            <div className="space-y-2">
              {send.text && <p className="whitespace-pre-wrap">{send.text}</p>}
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
          </div>
        </div>
      </div>,
    ];
    const row = rowForPendingSend(send, now);
    if (row) {
      nodes.push(renderTurnRow(`turn-row-${send.clientId}`, row));
    } else {
      nodes.push(
        <div
          key={`turn-row-${send.clientId}`}
          role="alert"
          className="flex items-center justify-end gap-2 text-sm text-destructive"
          data-testid="turn-not-delivered"
        >
          <span>{t("turn_not_delivered")}</span>
          {onRetrySend && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 px-2"
              onClick={() => onRetrySend(send.clientId)}
            >
              <RefreshCw className="mr-1 h-3.5 w-3.5" />
              {t("turn_retry_send")}
            </Button>
          )}
        </div>,
      );
    }
    return nodes;
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

  if (chatEvents.length === 0 && pendingSends.length === 0) {
    if (trailingStreamingWork) {
      return <div className="space-y-4">{trailingStreamingWork}</div>;
    }
    if (emptyState) {
      return (
        <div className="flex w-full flex-1 flex-col items-center justify-center px-4 py-8 text-center text-muted-foreground">
          {emptyState}
        </div>
      );
    }
    return (
      <div className="flex flex-col items-center justify-end text-center text-muted-foreground">
        <div className={chatSurfaceStyles.emptyStateCard}>
          <div className="mx-auto mb-4 flex h-10 w-10 items-center justify-center border border-border/70 bg-background text-muted-foreground">
            <AgentIcon className="h-5 w-5 opacity-65" />
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
      {(() => {
        const items: ReactNode[] = [];
        const nowMs = Date.now();
        let previousUserMs: number | undefined;
        const pushDaySeparator = (key: string, ts: number) => {
          if (needsDaySeparator(previousUserMs, ts)) {
            items.push(
              <div key={`day-${key}`} className={chatSurfaceStyles.daySeparator}>
                {formatDaySeparator(ts, nowMs, locale, {
                  today: t("day_today"),
                  yesterday: t("day_yesterday"),
                })}
              </div>,
            );
          }
          previousUserMs = ts;
        };
        for (const event of chatEvents) {
          const eventNode = ((): ReactNode => {
            if (event.type === "context.compacted") {
              const compactedData = getEventData(event, "context.compacted");
              return compactedData ? (
                <CompactionDivider key={event.id} data={compactedData} />
              ) : null;
            }

            if (event.type === "session.model.changed") {
              const modelChangedData = getEventData(event, "session.model.changed");
              return modelChangedData && !repeatedModelChangeIds.has(event.id) ? (
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
                  manageChatGptUsage={turnFailedData.error_code === "provider_usage_limit_reached"}
                />
              );
            }

            const sentMessage = getEventData(event, "conversation.message");
            if (sentMessage) {
              // What an explicit-communication agent sent with `send_message`: its reply,
              // shown like any assistant message. The tool call stays a work-log row.
              const text = sentMessage.text?.trim();
              if (!text) return null;
              const runs = runsByEventId?.get(event.id);
              const turnId = getKnownTurnId(event);
              return (
                <div
                  key={event.id}
                  className="chat-transcript-row space-y-2"
                  data-conversation-message-id={sentMessage.message_id}
                >
                  <div className="flex justify-start">
                    <div className={chatSurfaceStyles.agentMessageRow}>
                      <div className={chatSurfaceStyles.agentIcon}>
                        <AgentIcon className="h-3.5 w-3.5" />
                      </div>
                      <div className="flex flex-1 items-start gap-2">
                        <div className={chatSurfaceStyles.agentMessage}>
                          <MessageContent text={sentMessage.text} />
                        </div>
                        <MessageInfoIcon event={event} />
                      </div>
                    </div>
                  </div>
                  {runs ? <RunCards runs={runs} /> : null}
                  {turnId && (workLogTurnIds.has(turnId) || anchoredTurnIds.has(turnId))
                    ? null
                    : renderTurnDivider(
                        event.id,
                        turnDurationByEventId,
                        t("worked_for", {
                          duration: formatWorkedDuration(turnDurationByEventId.get(event.id) ?? 0),
                        }),
                        null,
                        t("trace_view_turn"),
                      )}
                </div>
              );
            }

            if (isWorkLogEvent(event)) {
              // Preserve event order and mount every loaded entry in testing/debugging views.
              if (!collapseWorkLog) return renderWorkLogEventContent(event, true);
              const turnId = getKnownTurnId(event);
              if (turnId && anchoredTurnIds.has(turnId)) return null;
              if (turnId) {
                const group = workLogEventsByTurnId.get(turnId) ?? [];
                if (group[0]?.id !== event.id) return null;
                return renderWorkLog(event, group, (isActive) => {
                  const entries = group
                    .map((groupEvent) => renderWorkLogEventContent(groupEvent, !isActive))
                    .filter(Boolean);
                  if (streamingRow && streamingWork?.turnId === turnId) entries.push(streamingRow);
                  return <WorkLogEntries entries={entries} />;
                });
              }

              return renderWorkLog(event, [event], (isActive) =>
                renderWorkLogEventContent(event, !isActive),
              );
            }

            const isUser = event.type === "input.message";
            const inputData = getEventData(event, "input.message");
            const outputData = getEventData(event, "output.message.completed");
            const data = inputData ?? outputData;
            if (!data) return null;

            const textContent = getMessageText(data);
            const outputError = outputData
              ? getRuntimeErrorFromOutputMessage(outputData)
              : undefined;
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
            // Platform-injected task updates (a thread finished, asked, or failed),
            // not words the person typed.
            const isTaskWake = isUser && data.message?.metadata?.everruns_origin === "task_wake";
            const taskUpdate = isTaskWake && textContent ? parseTaskUpdate(textContent) : null;
            // What a coordinator sent this thread, shown without the worker's instructions.
            const coordinatorMessage =
              isUser && !isTaskWake && textContent ? parseCoordinatorMessage(textContent) : null;
            const isToolOnlyMessage =
              !isUser && outputToolCalls.length > 0 && !textContent && images.length === 0;

            if (isToolOnlyMessage) {
              if (toolCalls.length === 0) return null;
              return renderWorkLog(
                event,
                [],
                () => (
                  <div className="space-y-1">
                    <ToolActivityGroup
                      toolCalls={toolCalls}
                      toolResultsMap={toolResultsMap}
                      toolProgressMap={toolProgressMap}
                      toolOutputMap={toolOutputMap}
                      approvalContexts={approvalContexts}
                    />
                  </div>
                ),
                toolCalls.filter((toolCall) => toolResultsMap.get(toolCall.id)?.error).length,
              );
            }

            return (
              <div
                key={isUser ? `user-${getClientMessageId(event) ?? event.id}` : event.id}
                className={`chat-transcript-row space-y-2 ${isUser ? "scroll-mt-4" : ""}`}
                data-message-anchor={isUser ? event.id : undefined}
                id={isUser ? `message-${data.message?.id}` : undefined}
              >
                {(textContent || images.length > 0) && (
                  <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                    {isUser ? (
                      <div className={chatSurfaceStyles.userMessage}>
                        {isTaskWake && (
                          <div className="mb-1 flex items-center gap-1 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                            <RefreshCw className="h-3 w-3" />
                            <span>{t("automatic_update")}</span>
                          </div>
                        )}
                        {isScheduleTriggered && (
                          <div className="mb-1 flex items-center gap-1 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                            <CalendarClock className="h-3 w-3" />
                            <span>{t("scheduled")}</span>
                          </div>
                        )}
                        {coordinatorMessage && (
                          <div className="mb-1 flex items-center gap-1 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                            <MessageSquare className="h-3 w-3" />
                            <span>{t("from_chat")}</span>
                          </div>
                        )}
                        <div className="flex items-start gap-2">
                          <div className="flex-1 space-y-2">
                            {taskUpdate ? (
                              <>
                                <p className="font-medium">
                                  {t(`task_update_${taskUpdate.kind}` as const, {
                                    title: taskUpdate.title,
                                  })}
                                </p>
                                {taskUpdate.body && (
                                  <p className="whitespace-pre-wrap">{taskUpdate.body}</p>
                                )}
                              </>
                            ) : coordinatorMessage ? (
                              <>
                                {coordinatorMessage.kind === "assignment" && (
                                  <p className="font-medium">
                                    {t("new_assignment", { title: coordinatorMessage.title })}
                                  </p>
                                )}
                                <p className="whitespace-pre-wrap">{coordinatorMessage.body}</p>
                              </>
                            ) : (
                              textContent && <p className="whitespace-pre-wrap">{textContent}</p>
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
                        </div>
                      </div>
                    ) : (
                      <div className={chatSurfaceStyles.agentMessageRow}>
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
                        <div
                          className={cn(
                            chatSurfaceStyles.agentActions,
                            event.id === latestAgentEventId ? "opacity-100" : "opacity-0",
                          )}
                          data-testid="agent-reply-actions"
                        >
                          {textContent && (
                            <CopyButton value={textContent} label={t("copy_message")} />
                          )}
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
                  <div className="space-y-1">
                    <ToolActivityGroup
                      toolCalls={toolCalls}
                      toolResultsMap={toolResultsMap}
                      toolProgressMap={toolProgressMap}
                      toolOutputMap={toolOutputMap}
                      approvalContexts={approvalContexts}
                    />
                  </div>
                )}

                {(() => {
                  const runs = runsByEventId?.get(event.id);
                  return runs ? <RunCards runs={runs} /> : null;
                })()}

                {(() => {
                  const turnId = getKnownTurnId(event);
                  if (turnId && (workLogTurnIds.has(turnId) || anchoredTurnIds.has(turnId))) {
                    return null;
                  }
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

          for (const marker of markersByEventId.get(event.id) ?? []) {
            items.push(renderParticipantMarker(marker));
          }
          if (event.type === "input.message" && eventNode) {
            pushDaySeparator(getClientMessageId(event) ?? event.id, Date.parse(event.ts));
          }
          items.push(eventNode);
          const turnRow = renderStoredTurnRow(event);
          if (turnRow) items.push(turnRow);
        }
        for (const send of pendingSends) {
          pushDaySeparator(send.clientId, send.sentAtMs);
          items.push(...renderPendingSend(send));
        }
        return items;
      })()}
      {trailingMarkers.map((marker) => renderParticipantMarker(marker))}
      {trailingStreamingWork}
    </div>
  );
});
