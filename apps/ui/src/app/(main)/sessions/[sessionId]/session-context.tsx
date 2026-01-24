"use client";

import { createContext, useContext, useState, useMemo, useEffect, type ReactNode } from "react";
import { useAgent, useSession, useEvents, useLlmModel } from "@/hooks";
import { sendUserMessage, cancelTurn } from "@/lib/api/sessions";
import { useMutation } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import type {
  Agent,
  Session,
  SessionStatus,
  Event,
  LlmModelWithProvider,
  Controls,
  ReasoningEffort,
  ToolCompletedData,
  InputMessageData,
  OutputMessageCompletedData,
  Message,
  TokenUsage,
  LlmGenerationData,
  ReasonThinkingStartedData,
  OutputMessageDeltaData,
} from "@/lib/api/types";
import { getTextFromContent, isToolCallPart } from "@/lib/api/types";
import type { UseMutationResult } from "@tanstack/react-query";

interface SessionContextValue {
  // IDs
  agentId: string | undefined;
  sessionId: string;
  // Data
  agent: Agent | undefined;
  session: Session | undefined;
  events: Event[] | undefined;
  llmModel: LlmModelWithProvider | undefined;
  chatEvents: Event[];
  toolResultsMap: Map<string, ToolCompletedData>;
  // Loading states
  sessionLoading: boolean;
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
  // Response waiting state
  isWaitingForResponse: boolean;
  setIsWaitingForResponse: (waiting: boolean) => void;
  // Streaming state (for real-time text updates)
  isThinking: boolean;
  streamingText: string | null;
  streamingTurnId: string | null;
  // Message sending
  sendMessage: UseMutationResult<
    Message,
    Error,
    { sessionId: string; content: string; controls?: Controls },
    { optimisticId: string; content: string }
  >;
  // Turn cancellation
  cancelCurrentTurn: UseMutationResult<void, Error, void, unknown>;
  // Utility functions
  getMessageText: (data: InputMessageData | OutputMessageCompletedData) => string;
  getToolCalls: (data: OutputMessageCompletedData) => Array<{ id: string; name: string; arguments: Record<string, unknown> }>;
}

const SessionContext = createContext<SessionContextValue | null>(null);

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
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  // Fetch session first to get agent_id
  const { data: session, isLoading: sessionLoading } = useSession(sessionId);

  // Derive agentId from session
  const agentId = session?.agent_id;

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

  // Optimistic events - shown immediately before SSE confirms
  const [optimisticEvents, setOptimisticEvents] = useState<Event[]>([]);

  // Custom sendMessage mutation with optimistic UI
  const sendMessage = useMutation({
    mutationFn: ({
      sessionId,
      content,
      controls,
    }: {
      sessionId: string;
      content: string;
      controls?: Controls;
    }) => sendUserMessage(org!, sessionId, content, controls),
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
        setOptimisticEvents((prev) =>
          prev.filter((e) => e.id !== context.optimisticId)
        );
      }
    },
  });

  // Cancel turn mutation
  const cancelCurrentTurn = useMutation({
    mutationFn: async () => {
      if (!org) throw new Error("Organization not found");
      await cancelTurn(org, sessionId);
    },
    onSuccess: () => {
      // Reset waiting state since turn is cancelled
      setIsWaitingForResponse(false);
      // Update local status to idle immediately
      setLocalStatus("idle");
    },
  });

  // Fetch LLM model info if session has a model_id
  const { data: llmModel } = useLlmModel(session?.model_id ?? "");

  // Fetch events using SSE - always enabled for real-time streaming
  // SSE handles backoff automatically (100ms → 10s when no new events)
  const { data: events, isLoading: eventsLoading } = useEvents(sessionId);

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
        break;
      }
      if (event.type === "turn.cancelled") {
        // Turn was cancelled - clear thinking state immediately
        setIsWaitingForResponse(false);
        setIsThinking(false);
        setStreamingText(null);
        setStreamingTurnId(null);
        break;
      }
    }
  }, [events]);

  // Update streaming state from SSE events (reason.thinking.started, output.message.delta, output.message.completed)
  // This provides real-time feedback while the LLM generates text
  useEffect(() => {
    if (!events || events.length === 0) return;

    // Process events from newest to oldest to find current streaming state
    for (let i = events.length - 1; i >= 0; i--) {
      const event = events[i];

      // output.message.completed finalizes the response - stop streaming
      if (event.type === "output.message.completed") {
        const turnId = event.context?.turn_id;
        // Only clear streaming if this message is for the current streaming turn
        if (!streamingTurnId || turnId === streamingTurnId) {
          setIsThinking(false);
          setStreamingText(null);
          setStreamingTurnId(null);
        }
        break;
      }

      // output.message.delta provides incremental text updates
      if (event.type === "output.message.delta") {
        const data = event.data as OutputMessageDeltaData;
        setIsThinking(false); // No longer just thinking, now we have text
        setStreamingText(data.accumulated);
        setStreamingTurnId(data.turn_id);
        break;
      }

      // reason.thinking.started indicates LLM is generating (before first text)
      if (event.type === "reason.thinking.started") {
        const data = event.data as ReasonThinkingStartedData;
        // Only set thinking if we don't already have streaming text for this turn
        if (!streamingText || streamingTurnId !== data.turn_id) {
          setIsThinking(true);
          setStreamingText(null);
          setStreamingTurnId(data.turn_id);
        }
        break;
      }
    }
  }, [events, streamingTurnId, streamingText]);

  // Reset local status, optimistic events, and streaming state when session changes
  useEffect(() => {
    setLocalStatus(null);
    setOptimisticEvents([]);
    setIsThinking(false);
    setStreamingText(null);
    setStreamingTurnId(null);
  }, [sessionId]);

  // Use local status if available, otherwise fall back to session status
  const effectiveStatus = localStatus ?? session?.status;

  // Determine if session is actively processing (only "active" means processing)
  const isActive = effectiveStatus === "active";

  // shouldPoll is no longer needed - we use SSE events for real-time status
  const shouldPoll = false;

  const [reasoningEffort, setReasoningEffort] = useState<ReasoningEffort | "">("");

  // Clean up optimistic events when real events arrive from SSE
  useEffect(() => {
    if (!events || events.length === 0 || optimisticEvents.length === 0) return;

    // Get text content from real user messages
    const realUserMessages = events
      .filter((e) => e.type === "input.message")
      .map((e) => {
        const data = e.data as InputMessageData;
        return getTextFromContent(data.message?.content || []);
      });

    // Remove optimistic events that have matching real events
    const optimisticToRemove = optimisticEvents.filter((optEvent) => {
      const data = optEvent.data as InputMessageData;
      const optText = getTextFromContent(data.message?.content || []);
      return realUserMessages.includes(optText);
    });

    if (optimisticToRemove.length > 0) {
      setOptimisticEvents((prev) =>
        prev.filter((e) => !optimisticToRemove.some((r) => r.id === e.id))
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
            e.type === "tool.completed"
        )
      : [];

    // Get text content from real user messages for deduplication
    const realUserTexts = new Set(
      realChatEvents
        .filter((e) => e.type === "input.message")
        .map((e) => {
          const data = e.data as InputMessageData;
          return getTextFromContent(data.message?.content || []);
        })
    );

    // Filter out optimistic events that already have a real counterpart
    const pendingOptimisticEvents = optimisticEvents.filter((optEvent) => {
      if (optEvent.type !== "input.message") return true;
      const data = optEvent.data as InputMessageData;
      const optText = getTextFromContent(data.message?.content || []);
      return !realUserTexts.has(optText);
    });

    return [...realChatEvents, ...pendingOptimisticEvents];
  }, [events, optimisticEvents]);

  // Build tool result lookup by tool_call_id
  const toolResultsMap = useMemo(() => {
    const map = new Map<string, ToolCompletedData>();
    if (!events) return map;
    for (const event of events) {
      if (event.type === "tool.completed") {
        const data = event.data as ToolCompletedData;
        map.set(data.tool_call_id, data);
      }
    }
    return map;
  }, [events]);

  // Compute live usage with real-time updates during turns
  const liveUsage = useMemo((): TokenUsage | undefined => {
    if (!events || events.length === 0) {
      return session?.usage;
    }

    // Sum ALL llm.generation events to get cumulative usage
    let inputTokens = 0;
    let outputTokens = 0;
    let cacheReadTokens = 0;
    let cacheCreationTokens = 0;
    let hasLlmEvents = false;

    for (const event of events) {
      if (event.type === "llm.generation") {
        const data = event.data as LlmGenerationData;
        if (data.metadata?.usage) {
          hasLlmEvents = true;
          inputTokens += data.metadata.usage.input_tokens;
          outputTokens += data.metadata.usage.output_tokens;
          cacheReadTokens += data.metadata.usage.cache_read_tokens ?? 0;
          cacheCreationTokens += data.metadata.usage.cache_creation_tokens ?? 0;
        }
      }
    }

    // If we have llm.generation events, use their sum as the source of truth
    if (hasLlmEvents) {
      return {
        input_tokens: inputTokens,
        output_tokens: outputTokens,
        cache_read_tokens: cacheReadTokens > 0 ? cacheReadTokens : undefined,
        cache_creation_tokens: cacheCreationTokens > 0 ? cacheCreationTokens : undefined,
      };
    }

    // Fall back to session usage
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

  // Extract text from message event data
  const getMessageText = (data: InputMessageData | OutputMessageCompletedData): string => {
    const content = data.message?.content;
    if (!content) return "";
    return getTextFromContent(content);
  };

  // Get tool calls from message event data
  const getToolCalls = (
    data: OutputMessageCompletedData
  ): Array<{ id: string; name: string; arguments: Record<string, unknown> }> => {
    const content = data.message?.content;
    if (!content) return [];
    return content
      .filter(isToolCallPart)
      .map((part) => ({ id: part.id, name: part.name, arguments: part.arguments }));
  };

  const value: SessionContextValue = {
    agentId,
    sessionId,
    agent,
    session,
    events,
    llmModel,
    chatEvents,
    toolResultsMap,
    sessionLoading,
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
    isWaitingForResponse,
    setIsWaitingForResponse,
    isThinking,
    streamingText,
    streamingTurnId,
    sendMessage,
    cancelCurrentTurn,
    getMessageText,
    getToolCalls,
  };

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}
