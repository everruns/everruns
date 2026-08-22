"use client";

import {
  createContext,
  useContext,
  useState,
  useMemo,
  useCallback,
  useEffect,
  useRef,
  type ReactNode,
} from "react";
import { useAgent, useSession, useEvents, useModel, useSessionResolvedModel } from "@/hooks";
import { sendUserMessage, cancelTurn } from "@/lib/api/sessions";
import { useMutation } from "@tanstack/react-query";
import { usePathname } from "next/navigation";
import { isChatThread } from "@/lib/chat-threads";
import { useOrg } from "@/providers/org-provider";
import { useLocale } from "@/providers/locale-provider";
import type {
  Agent,
  Session,
  SessionStatus,
  Event,
  ModelWithProvider,
  Controls,
  ReasoningEffort,
  Verbosity,
  ToolCompletedData,
  ToolProgressData,
  InputMessageData,
  OutputMessageCompletedData,
  Message,
  TokenUsage,
} from "@/lib/api/types";
import { getTextFromContent, isToolCallPart, getEventData } from "@/lib/api/types";
import { getLocalizedOutputMessageText } from "@/lib/runtime-errors";
import { latestStreamingMessage } from "@/lib/streaming-message-state";
import type { UseMutationResult } from "@tanstack/react-query";
import { useWebMcpTool } from "@/hooks/use-webmcp-tool";
import { useWebMcp } from "@/providers/webmcp-context";
import type { WebMcpToolDefinition } from "@/lib/webmcp/types";

/** Accumulated streamed output for a single tool call */
export interface ToolOutputStreams {
  stdout: string;
  stderr: string;
}

export interface SessionContextValue {
  // IDs
  agentId: string | undefined;
  sessionId: string;
  // Data
  agent: Agent | undefined;
  session: Session | undefined;
  events: Event[] | undefined;
  llmModel: ModelWithProvider | undefined;
  chatEvents: Event[];
  toolResultsMap: Map<string, ToolCompletedData>;
  toolProgressMap: Map<string, ToolProgressData>;
  toolOutputMap: Map<string, ToolOutputStreams>;
  // Loading states
  sessionLoading: boolean;
  llmModelLoading: boolean;
  eventsLoading: boolean;
  // Derived states (updated via SSE)
  effectiveStatus: SessionStatus | undefined;
  liveUsage: TokenUsage | undefined;
  isActive: boolean;
  shouldPoll: boolean;
  supportsReasoning: boolean;
  // Reasoning effort
  reasoningEffort: ReasoningEffort | "";
  setReasoningEffort: (effort: ReasoningEffort | "") => void;
  getReasoningEffortName: (value: string) => string;
  defaultEffortName: string;
  // Verbosity
  verbosity: Verbosity | "";
  setVerbosity: (value: Verbosity | "") => void;
  // Response waiting state
  isWaitingForResponse: boolean;
  setIsWaitingForResponse: (waiting: boolean) => void;
  // Streaming state (for real-time text updates)
  isThinking: boolean;
  streamingText: string | null;
  streamingTurnId: string | null;
  streamingMessageId: string | null;
  /** Current iteration number within the active turn (1-based) */
  streamingIteration: number | null;
  // Message sending
  sendMessage: UseMutationResult<
    Message,
    Error,
    {
      sessionId: string;
      content: string;
      controls?: Controls;
      addressedParticipantId?: string | null;
    },
    { optimisticId: string; content: string }
  >;
  // Turn cancellation
  cancelCurrentTurn: UseMutationResult<void, Error, void, unknown>;
  // Pagination (load older events on scroll up)
  hasMoreEvents: boolean;
  loadingOlderEvents: boolean;
  loadOlderEvents: () => Promise<void>;
  totalNonDeltaCount: number | undefined;
  // Utility functions
  getMessageText: (data: InputMessageData | OutputMessageCompletedData) => string;
  getToolCalls: (
    data: OutputMessageCompletedData,
  ) => Array<{ id: string; name: string; arguments: Record<string, unknown> }>;
}

export const SessionContext = createContext<SessionContextValue | null>(null);

export function useSessionContext() {
  const context = useContext(SessionContext);
  if (!context) {
    throw new Error("useSessionContext must be used within a SessionProvider");
  }
  return context;
}

interface SessionProviderProps {
  sessionId: string;
  children: ReactNode;
}

// Session provider that derives agentId from the session (for org-level routes)
export function SessionProvider({ sessionId, children }: SessionProviderProps) {
  const pathname = usePathname();
  const { currentOrg } = useOrg();
  const webmcp = useWebMcp();
  const { locale } = useLocale();
  const org = currentOrg?.public_id;

  // Fetch session first to get agent_id
  const { data: session, isLoading: sessionLoading } = useSession(sessionId);

  // Derive agentId from session (convert null to undefined)
  const agentId = session?.agent_id ?? undefined;

  // Fetch agent using derived agentId
  const { data: agent } = useAgent(agentId ?? "");

  // Track if user has sent a message and is waiting for response
  const [isWaitingForResponse, setIsWaitingForResponse] = useState(false);

  // Track session status locally based on SSE events (no polling needed)
  const [localStatus, setLocalStatus] = useState<SessionStatus | null>(null);

  // Streaming state - tracks real-time text generation
  const [isThinking, setIsThinking] = useState(false);
  const [streamingText, setStreamingText] = useState<string | null>(null);
  const [streamingTurnId, setStreamingTurnId] = useState<string | null>(null);
  const [streamingMessageId, setStreamingMessageId] = useState<string | null>(null);
  const [streamingIteration, setStreamingIteration] = useState<number | null>(null);

  // Optimistic events - shown immediately before SSE confirms
  const [optimisticEvents, setOptimisticEvents] = useState<Event[]>([]);
  // THREAT[TM-WEB-017]: reject concurrent non-idempotent browser-agent mutations.
  const webMcpActionPendingRef = useRef(false);

  // Custom sendMessage mutation with optimistic UI
  const sendMessage = useMutation({
    mutationFn: ({
      sessionId,
      content,
      controls,
      addressedParticipantId,
    }: {
      sessionId: string;
      content: string;
      controls?: Controls;
      addressedParticipantId?: string | null;
    }) => sendUserMessage(sessionId, content, controls, addressedParticipantId),
    onMutate: async ({ sessionId, content }) => {
      // Create optimistic event immediately
      const optimisticId = `optimistic-${Date.now()}`;
      const optimisticEvent: Event = {
        id: optimisticId,
        type: "input.message",
        ts: new Date().toISOString(),
        session_id: sessionId,
        context: {},
        data: {
          message: {
            id: optimisticId,
            session_id: sessionId,
            sequence: -1, // Will be replaced by real sequence
            role: "user" as const,
            content: [{ type: "text" as const, text: content }],
            tool_call_id: null,
            created_at: new Date().toISOString(),
          },
        },
      };
      setOptimisticEvents((prev) => [...prev, optimisticEvent]);
      return { optimisticId, content };
    },
    // Don't remove optimistic event on success - wait for SSE to deliver real event
    onError: (_error, _variables, context) => {
      // Only remove optimistic event on error
      if (context?.optimisticId) {
        setOptimisticEvents((prev) => prev.filter((e) => e.id !== context.optimisticId));
      }
    },
  });

  // Cancel turn mutation
  const cancelCurrentTurn = useMutation({
    mutationFn: async () => {
      if (!org) throw new Error("Organization not found");
      await cancelTurn(sessionId);
    },
    onSuccess: () => {
      // Reset waiting state since turn is cancelled
      setIsWaitingForResponse(false);
      // Update local status to idle immediately
      setLocalStatus("idle");
    },
  });

  // The server owns model precedence; the UI resolves only the returned model resource.
  const { data: resolvedModel, isLoading: resolvedModelLoading } =
    useSessionResolvedModel(sessionId);
  const { data: llmModel, isLoading: modelLoading } = useModel(resolvedModel?.model_id ?? "");
  const llmModelLoading = resolvedModelLoading || modelLoading;

  // Fetch events using paginated REST + SSE for real-time streaming
  const {
    data: events,
    isLoading: eventsLoading,
    hasMore: hasMoreEvents,
    loadingOlder: loadingOlderEvents,
    loadOlderEvents,
    totalNonDeltaCount,
  } = useEvents(sessionId);

  // Update local status from SSE events (session.activated, session.idled)
  useEffect(() => {
    if (!events || events.length === 0) return;

    // Find the most recent session status event
    for (let i = events.length - 1; i >= 0; i--) {
      const event = events[i];
      if (event.type === "session.activated") {
        setLocalStatus("active");
        break;
      }
      if (event.type === "session.idled") {
        setLocalStatus("idle");
        // When session becomes idle, user is no longer waiting for response
        setIsWaitingForResponse(false);
        // Also clear thinking/streaming state - session is done
        setIsThinking(false);
        setStreamingText(null);
        setStreamingTurnId(null);
        setStreamingMessageId(null);
        break;
      }
      if (event.type === "turn.cancelled") {
        // Turn was cancelled - clear thinking state immediately
        setIsWaitingForResponse(false);
        setIsThinking(false);
        setStreamingText(null);
        setStreamingTurnId(null);
        setStreamingMessageId(null);
        break;
      }
      if (event.type === "turn.failed") {
        // Turn failed - clear optimistic waiting/streaming state immediately.
        setIsWaitingForResponse(false);
        setIsThinking(false);
        setStreamingText(null);
        setStreamingTurnId(null);
        setStreamingMessageId(null);
        setStreamingIteration(null);
        setLocalStatus("idle");
        break;
      }
    }
  }, [events]);

  // Project streaming state by message_id. A single turn may contain multiple
  // commentary/final assistant messages, each with an independent lifecycle.
  useEffect(() => {
    const streaming = latestStreamingMessage(events ?? []);
    setIsThinking(streaming.isThinking);
    setStreamingText(streaming.text);
    setStreamingTurnId(streaming.turnId);
    setStreamingMessageId(streaming.messageId);
    setStreamingIteration(streaming.iteration);
  }, [events]);

  // Reset local status, optimistic events, and streaming state when session changes
  useEffect(() => {
    setLocalStatus(null);
    setOptimisticEvents([]);
    setIsThinking(false);
    setStreamingText(null);
    setStreamingTurnId(null);
    setStreamingMessageId(null);
    setStreamingIteration(null);
  }, [sessionId]);

  // Use local status if available, otherwise fall back to session status
  const effectiveStatus = localStatus ?? session?.status;

  // Determine if session is actively processing
  // "active" = turn running, "waiting_for_tool_results" = paused for client-side tool
  const isActive = effectiveStatus === "active" || effectiveStatus === "waiting_for_tool_results";
  // Browser-agent messaging is offered only on a surface that has a composer.
  // Session detail is a read-only recording (EVE-854), so registering these
  // tools there would hand a browser agent a mutation the page itself refuses.
  const isChatSurface =
    !!session &&
    isChatThread(session) &&
    (pathname === "/chat" || pathname === `/chats/${sessionId}`);
  const canSendWebMcpMessage = effectiveStatus === "idle" || effectiveStatus === "started";

  const sendMessageTool = useMemo<WebMcpToolDefinition>(
    () => ({
      name: "everruns_send_message",
      description: "Send a text message in the Everruns session displayed on this page.",
      inputSchema: {
        type: "object",
        properties: {
          message: { type: "string", description: "Text to send to the session agent." },
        },
        required: ["message"],
        additionalProperties: false,
      },
      annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false },
      execute: async (input) => {
        webmcp.assertBinding(webmcp.bindingToken);
        if (!session || session.id !== sessionId || !canSendWebMcpMessage || !isChatSurface) {
          throw new DOMException(
            "The bound session is no longer ready for a message",
            "AbortError",
          );
        }
        if (webMcpActionPendingRef.current || sendMessage.isPending) {
          throw new Error("Another session action is already running");
        }
        if (typeof input.message !== "string" || !input.message.trim()) {
          throw new TypeError("message must be a non-empty string");
        }
        const message = input.message.trim().slice(0, 8_000);
        await webmcp.requestApproval({
          title: "Send this agent message?",
          description: `Send to ${session.title || "this session"}: “${message}” This starts a potentially billable agent turn.`,
          confirmLabel: "Send message",
        });
        webmcp.assertBinding(webmcp.bindingToken);
        webMcpActionPendingRef.current = true;
        try {
          const created = await sendMessage.mutateAsync({ sessionId, content: message });
          return { sent: true, message_id: created.id, session_id: sessionId };
        } finally {
          webMcpActionPendingRef.current = false;
        }
      },
    }),
    [canSendWebMcpMessage, isChatSurface, sendMessage, session, sessionId, webmcp],
  );

  const cancelTurnTool = useMemo<WebMcpToolDefinition>(
    () => ({
      name: "everruns_cancel_turn",
      description:
        "Cancel the currently active turn in the Everruns session displayed on this page.",
      inputSchema: { type: "object", properties: {}, additionalProperties: false },
      annotations: { readOnlyHint: false, destructiveHint: true, idempotentHint: false },
      execute: async () => {
        webmcp.assertBinding(webmcp.bindingToken);
        if (!session || session.id !== sessionId || !isActive || !isChatSurface) {
          throw new DOMException("The bound session no longer has an active turn", "AbortError");
        }
        if (webMcpActionPendingRef.current || cancelCurrentTurn.isPending) {
          throw new Error("Another session action is already running");
        }
        await webmcp.requestApproval({
          title: "Cancel the active turn?",
          description: `Stop the running turn in ${session.title || "this session"}. Partial work may be lost.`,
          confirmLabel: "Cancel turn",
          destructive: true,
        });
        webmcp.assertBinding(webmcp.bindingToken);
        webMcpActionPendingRef.current = true;
        try {
          await cancelCurrentTurn.mutateAsync();
          return { cancelled: true, session_id: sessionId };
        } finally {
          webMcpActionPendingRef.current = false;
        }
      },
    }),
    [cancelCurrentTurn, isActive, isChatSurface, session, sessionId, webmcp],
  );

  useWebMcpTool(sendMessageTool, {
    enabled: isChatSurface && canSendWebMcpMessage,
    scopeKey: `${sessionId}:${effectiveStatus}`,
  });
  useWebMcpTool(cancelTurnTool, {
    enabled: isChatSurface && isActive,
    scopeKey: `${sessionId}:${effectiveStatus}`,
  });

  // shouldPoll is no longer needed - we use SSE events for real-time status
  const shouldPoll = false;

  const [reasoningEffort, setReasoningEffort] = useState<ReasoningEffort | "">("");
  const [verbosity, setVerbosity] = useState<Verbosity | "">("");

  // Clean up optimistic events when real events arrive from SSE
  useEffect(() => {
    if (!events || events.length === 0 || optimisticEvents.length === 0) return;

    // Get text content from real user messages
    const realUserMessages = events
      .filter((e) => e.type === "input.message")
      .map((e) => {
        const data = getEventData(e, "input.message");
        return getTextFromContent(data?.message?.content || []);
      });

    // Remove optimistic events that have matching real events
    const optimisticToRemove = optimisticEvents.filter((optEvent) => {
      const data = getEventData(optEvent, "input.message");
      const optText = getTextFromContent(data?.message?.content || []);
      return realUserMessages.includes(optText);
    });

    if (optimisticToRemove.length > 0) {
      setOptimisticEvents((prev) =>
        prev.filter((e) => !optimisticToRemove.some((r) => r.id === e.id)),
      );
    }
  }, [events, optimisticEvents]);

  // Filter chat-relevant events and merge with optimistic events
  const chatEvents = useMemo(() => {
    const realChatEvents = events
      ? events.filter(
          (e) =>
            e.type === "input.message" ||
            e.type === "output.message.completed" ||
            e.type === "turn.failed" ||
            e.type === "reason.completed" ||
            e.type === "reason.item" ||
            e.type === "act.started" ||
            e.type === "act.completed" ||
            e.type === "tool.started" ||
            e.type === "tool.progress" ||
            e.type === "tool.completed" ||
            e.type === "tool.call_requested" ||
            e.type === "context.compacted",
        )
      : [];

    // Get text content from real user messages for deduplication
    const realUserTexts = new Set(
      realChatEvents
        .filter((e) => e.type === "input.message")
        .map((e) => {
          const data = getEventData(e, "input.message");
          return getTextFromContent(data?.message?.content || []);
        }),
    );

    // Filter out optimistic events that already have a real counterpart
    const pendingOptimisticEvents = optimisticEvents.filter((optEvent) => {
      if (optEvent.type !== "input.message") return true;
      const data = getEventData(optEvent, "input.message");
      const optText = getTextFromContent(data?.message?.content || []);
      return !realUserTexts.has(optText);
    });

    const merged = [...realChatEvents, ...pendingOptimisticEvents];
    // Sort by sequence to guarantee correct chronological order regardless of
    // SSE arrival order. Only optimistic events use sequence -1, and they
    // should appear at the end (they represent the latest user message).
    // Events missing a sequence fall back to timestamp ordering.
    merged.sort((a, b) => {
      const seqA = a.sequence;
      const seqB = b.sequence;
      const isOptimisticA = seqA === -1;
      const isOptimisticB = seqB === -1;
      const hasSeqA = seqA != null && seqA !== -1;
      const hasSeqB = seqB != null && seqB !== -1;

      // Optimistic events always sort to the end
      if (isOptimisticA !== isOptimisticB) {
        return isOptimisticA ? 1 : -1;
      }

      // Both have real sequences — sort numerically
      if (hasSeqA && hasSeqB && seqA !== seqB) {
        return seqA - seqB;
      }

      // One has a sequence and the other doesn't — sequenced first
      if (hasSeqA !== hasSeqB) {
        return hasSeqA ? -1 : 1;
      }

      // Same sequence, both missing, or both optimistic — compare by timestamp
      return a.ts.localeCompare(b.ts);
    });
    return merged;
  }, [events, optimisticEvents]);

  // Build tool result lookup by tool_call_id
  const toolResultsMap = useMemo(() => {
    const map = new Map<string, ToolCompletedData>();
    if (!events) return map;
    for (const event of events) {
      const data = getEventData(event, "tool.completed");
      if (data) {
        map.set(data.tool_call_id, data);
      }
    }
    return map;
  }, [events]);

  // Build latest tool progress lookup by tool_call_id (keeps last progress per tool)
  const toolProgressMap = useMemo(() => {
    const map = new Map<string, ToolProgressData>();
    if (!events) return map;
    for (const event of events) {
      const data = getEventData(event, "tool.progress");
      if (data) {
        map.set(data.tool_call_id, data);
      }
    }
    return map;
  }, [events]);

  // Accumulate streamed tool output by tool_call_id.
  // Uses chunk arrays and joins once per tool to avoid O(n²) string concatenation.
  const toolOutputMap = useMemo(() => {
    const chunkMap = new Map<string, { stdoutChunks: string[]; stderrChunks: string[] }>();
    if (!events) return new Map<string, ToolOutputStreams>();
    for (const event of events) {
      const data = getEventData(event, "tool.output.delta");
      if (!data) continue;
      const existing = chunkMap.get(data.tool_call_id) ?? { stdoutChunks: [], stderrChunks: [] };
      if (data.stream === "stderr") {
        existing.stderrChunks.push(data.delta);
      } else {
        // stdout and any unknown streams default to stdout
        existing.stdoutChunks.push(data.delta);
      }
      chunkMap.set(data.tool_call_id, existing);
    }
    const result = new Map<string, ToolOutputStreams>();
    for (const [toolCallId, streams] of chunkMap.entries()) {
      result.set(toolCallId, {
        stdout: streams.stdoutChunks.join(""),
        stderr: streams.stderrChunks.join(""),
      });
    }
    return result;
  }, [events]);

  // Incremental token usage accumulation — O(1) per new event instead of O(n).
  // Tracks how many events have been processed so only new events are scanned.
  const usageRef = useRef({
    sessionId: session?.id,
    maxSequenceProcessed: -1,
    processedIdsWithoutSequence: new Set<string>(),
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    hasLlmEvents: false,
  });

  // Reset accumulator when switching sessions.
  if (usageRef.current.sessionId !== session?.id) {
    usageRef.current = {
      sessionId: session?.id,
      maxSequenceProcessed: -1,
      processedIdsWithoutSequence: new Set<string>(),
      inputTokens: 0,
      outputTokens: 0,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      hasLlmEvents: false,
    };
  }

  const liveUsage = useMemo((): TokenUsage | undefined => {
    if (!events || events.length === 0) {
      return session?.usage;
    }

    const acc = usageRef.current;

    // Scan only events that are newer than what we've already accumulated.
    // Keep unsequenced processed IDs bounded to the current in-memory event window.
    const retainedUnsequencedIds = new Set<string>();
    for (const event of events) {
      const eventSequence = event.sequence ?? null;
      if (eventSequence == null) {
        retainedUnsequencedIds.add(event.id);
      }
      if (eventSequence != null && eventSequence <= acc.maxSequenceProcessed) {
        continue;
      }
      if (eventSequence == null && acc.processedIdsWithoutSequence.has(event.id)) {
        continue;
      }

      const llmData = getEventData(event, "llm.generation");
      if (llmData?.metadata?.usage) {
        acc.hasLlmEvents = true;
        acc.inputTokens += llmData.metadata.usage.input_tokens;
        acc.outputTokens += llmData.metadata.usage.output_tokens;
        acc.cacheReadTokens += llmData.metadata.usage.cache_read_tokens ?? 0;
        acc.cacheCreationTokens += llmData.metadata.usage.cache_creation_tokens ?? 0;
      }

      if (eventSequence != null) {
        acc.maxSequenceProcessed = Math.max(acc.maxSequenceProcessed, eventSequence);
      } else {
        acc.processedIdsWithoutSequence.add(event.id);
      }
    }

    acc.processedIdsWithoutSequence = retainedUnsequencedIds;

    if (acc.hasLlmEvents) {
      return {
        input_tokens: acc.inputTokens,
        output_tokens: acc.outputTokens,
        cache_read_tokens: acc.cacheReadTokens > 0 ? acc.cacheReadTokens : undefined,
        cache_creation_tokens: acc.cacheCreationTokens > 0 ? acc.cacheCreationTokens : undefined,
      };
    }

    return session?.usage;
  }, [events, session?.usage]);

  // Check if the model supports reasoning effort
  const supportsReasoning = !!(llmModel?.profile?.reasoning && llmModel?.profile?.reasoning_effort);
  const reasoningEffortConfig = llmModel?.profile?.reasoning_effort;

  // Get display name for a reasoning effort value
  const getReasoningEffortName = (value: string): string => {
    const effort = reasoningEffortConfig?.values.find((e) => e.value === value);
    return effort?.name ?? value;
  };

  // Get the default effort display name
  const defaultEffortName = reasoningEffortConfig?.default
    ? getReasoningEffortName(reasoningEffortConfig.default)
    : "Medium";

  // Extract text from message event data (stable ref — no deps)
  const getMessageText = useCallback(
    (data: InputMessageData | OutputMessageCompletedData): string => {
      const content = data.message?.content;
      if (!content) return "";
      return data.message?.role === "agent"
        ? getLocalizedOutputMessageText(locale, data as OutputMessageCompletedData)
        : getTextFromContent(content);
    },
    [locale],
  );

  // Get tool calls from message event data (stable ref — no deps)
  const getToolCalls = useCallback(
    (
      data: OutputMessageCompletedData,
    ): Array<{
      id: string;
      name: string;
      arguments: Record<string, unknown>;
    }> => {
      const content = data.message?.content;
      if (!content) return [];
      return content.filter(isToolCallPart).map((part) => ({
        id: part.id,
        name: part.name,
        arguments: part.arguments,
      }));
    },
    [],
  );

  const value: SessionContextValue = {
    agentId,
    sessionId,
    agent,
    session,
    events,
    llmModel,
    chatEvents,
    toolResultsMap,
    toolProgressMap,
    toolOutputMap,
    sessionLoading,
    llmModelLoading,
    eventsLoading,
    effectiveStatus,
    liveUsage,
    isActive,
    shouldPoll,
    supportsReasoning,
    reasoningEffort,
    setReasoningEffort,
    getReasoningEffortName,
    defaultEffortName,
    verbosity,
    setVerbosity,
    isWaitingForResponse,
    setIsWaitingForResponse,
    isThinking,
    streamingText,
    streamingTurnId,
    streamingMessageId,
    streamingIteration,
    sendMessage,
    cancelCurrentTurn,
    hasMoreEvents,
    loadingOlderEvents,
    loadOlderEvents,
    totalNonDeltaCount,
    getMessageText,
    getToolCalls,
  };

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}
