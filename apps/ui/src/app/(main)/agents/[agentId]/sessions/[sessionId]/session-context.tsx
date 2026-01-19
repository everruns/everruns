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
  ToolCallCompletedData,
  MessageUserData,
  MessageAgentData,
  Message,
  TokenUsage,
  SessionIdledData,
  LlmGenerationData,
  AgentThinkingData,
  TextDeltaData,
} from "@/lib/api/types";
import { getTextFromContent, isToolCallPart } from "@/lib/api/types";
import type { UseMutationResult } from "@tanstack/react-query";

interface SessionContextValue {
  // IDs
  agentId: string;
  sessionId: string;
  // Data
  agent: Agent | undefined;
  session: Session | undefined;
  events: Event[] | undefined;
  llmModel: LlmModelWithProvider | undefined;
  chatEvents: Event[];
  toolResultsMap: Map<string, ToolCallCompletedData>;
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
    { agentId: string; sessionId: string; content: string; controls?: Controls },
    { optimisticId: string; content: string }
  >;
  // Turn cancellation
  cancelCurrentTurn: UseMutationResult<void, Error, void, unknown>;
  // Utility functions
  getMessageText: (data: MessageUserData | MessageAgentData) => string;
  getToolCalls: (data: MessageAgentData) => Array<{ id: string; name: string; arguments: Record<string, unknown> }>;
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
  agentId: string;
  sessionId: string;
  children: ReactNode;
}

export function SessionProvider({ agentId, sessionId, children }: SessionProviderProps) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  const { data: agent } = useAgent(agentId);

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

  // Fetch session once to get initial data
  const { data: session, isLoading: sessionLoading } = useSession(agentId, sessionId);

  // Custom sendMessage mutation with optimistic UI
  const sendMessage = useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      content,
      controls,
    }: {
      agentId: string;
      sessionId: string;
      content: string;
      controls?: Controls;
    }) => sendUserMessage(org!, agentId, sessionId, content, controls),
    onMutate: async ({ sessionId, content }) => {
      // Create optimistic event immediately
      const optimisticId = `optimistic-${Date.now()}`;
      const optimisticEvent: Event = {
        id: optimisticId,
        type: "message.user",
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
      await cancelTurn(org, agentId, sessionId);
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
  const { data: events, isLoading: eventsLoading } = useEvents(agentId, sessionId);

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

  // Update streaming state from SSE events (agent.thinking, text.delta, message.agent)
  // This provides real-time feedback while the LLM generates text
  useEffect(() => {
    if (!events || events.length === 0) return;

    // Process events from newest to oldest to find current streaming state
    for (let i = events.length - 1; i >= 0; i--) {
      const event = events[i];

      // message.agent finalizes the response - stop streaming
      if (event.type === "message.agent") {
        const turnId = event.context?.turn_id;
        // Only clear streaming if this message is for the current streaming turn
        if (!streamingTurnId || turnId === streamingTurnId) {
          setIsThinking(false);
          setStreamingText(null);
          setStreamingTurnId(null);
        }
        break;
      }

      // text.delta provides incremental text updates
      if (event.type === "text.delta") {
        const data = event.data as TextDeltaData;
        setIsThinking(false); // No longer just thinking, now we have text
        setStreamingText(data.accumulated);
        setStreamingTurnId(data.turn_id);
        break;
      }

      // agent.thinking indicates LLM is generating (before first text)
      if (event.type === "agent.thinking") {
        const data = event.data as AgentThinkingData;
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
      .filter((e) => e.type === "message.user")
      .map((e) => {
        const data = e.data as MessageUserData;
        return getTextFromContent(data.message?.content || []);
      });

    // Remove optimistic events that have matching real events
    const optimisticToRemove = optimisticEvents.filter((optEvent) => {
      const data = optEvent.data as MessageUserData;
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
            e.type === "message.user" ||
            e.type === "message.agent" ||
            e.type === "tool.call_completed"
        )
      : [];

    // Get text content from real user messages for deduplication
    const realUserTexts = new Set(
      realChatEvents
        .filter((e) => e.type === "message.user")
        .map((e) => {
          const data = e.data as MessageUserData;
          return getTextFromContent(data.message?.content || []);
        })
    );

    // Filter out optimistic events that already have a real counterpart
    const pendingOptimisticEvents = optimisticEvents.filter((optEvent) => {
      if (optEvent.type !== "message.user") return true;
      const data = optEvent.data as MessageUserData;
      const optText = getTextFromContent(data.message?.content || []);
      return !realUserTexts.has(optText);
    });

    return [...realChatEvents, ...pendingOptimisticEvents];
  }, [events, optimisticEvents]);

  // Build tool result lookup by tool_call_id
  const toolResultsMap = useMemo(() => {
    const map = new Map<string, ToolCallCompletedData>();
    if (!events) return map;
    for (const event of events) {
      if (event.type === "tool.call_completed") {
        const data = event.data as ToolCallCompletedData;
        map.set(data.tool_call_id, data);
      }
    }
    return map;
  }, [events]);

  // Compute live usage with real-time updates during turns:
  // 1. session.idled - sets the baseline (cumulative from backend)
  // 2. llm.generation - adds to counters during turn execution
  // 3. session.idled - resets to final cumulative value
  const liveUsage = useMemo((): TokenUsage | undefined => {
    if (!events || events.length === 0) {
      return session?.usage;
    }

    // Find the latest session.idled event (regardless of whether it has usage)
    // This marks the boundary - we only count llm.generation events AFTER this
    let latestIdledIndex = -1;
    let latestIdledUsage: TokenUsage | undefined;

    for (let i = events.length - 1; i >= 0; i--) {
      if (events[i].type === "session.idled") {
        latestIdledIndex = i;
        latestIdledUsage = (events[i].data as SessionIdledData).usage;
        break; // Always use the latest session.idled, even if usage is null
      }
    }

    // Sum ALL llm.generation events to get cumulative usage
    // This is the source of truth for total tokens consumed
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

    // Fall back to session.idled usage or initial session usage
    return latestIdledUsage ?? session?.usage;
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
  const getMessageText = (data: MessageUserData | MessageAgentData): string => {
    const content = data.message?.content;
    if (!content) return "";
    return getTextFromContent(content);
  };

  // Get tool calls from message event data
  const getToolCalls = (
    data: MessageAgentData
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
