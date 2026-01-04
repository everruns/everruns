"use client";

import { createContext, useContext, useState, useMemo, type ReactNode } from "react";
import { useAgent, useSession, useEvents, useSendMessage, useLlmModel } from "@/hooks";
import type {
  Agent,
  Session,
  Event,
  LlmModelWithProvider,
  Controls,
  ReasoningEffort,
  ToolCallCompletedData,
  MessageUserData,
  MessageAgentData,
  Message,
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
  // Derived states
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
  // Message sending
  sendMessage: UseMutationResult<
    Message,
    Error,
    { agentId: string; sessionId: string; content: string; controls?: Controls },
    unknown
  >;
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
  const { data: agent } = useAgent(agentId);

  // Track if user has sent a message and is waiting for response
  const [isWaitingForResponse, setIsWaitingForResponse] = useState(false);

  // First fetch session without polling to get initial status
  const { data: session, isLoading: sessionLoading } = useSession(agentId, sessionId);
  const sendMessage = useSendMessage();

  // Fetch LLM model info if session has a model_id
  const { data: llmModel } = useLlmModel(session?.model_id ?? "");

  // Determine if session is still processing
  const isActive = session?.status === "running" || session?.status === "pending";

  // Derive whether we should poll - only when waiting AND session is active
  const shouldPoll = isWaitingForResponse && isActive;

  // Re-fetch session with polling when shouldPoll changes
  useSession(agentId, sessionId, {
    refetchInterval: shouldPoll ? 1000 : false,
  });

  // Fetch events - used for both chat rendering and events tab
  const { data: events, isLoading: eventsLoading } = useEvents(agentId, sessionId, {
    refetchInterval: shouldPoll ? 1000 : false,
  });

  const [reasoningEffort, setReasoningEffort] = useState<ReasoningEffort | "">("");

  // Filter chat-relevant events
  const chatEvents = useMemo(() => {
    if (!events) return [];
    return events.filter(
      (e) =>
        e.type === "message.user" || e.type === "message.agent" || e.type === "tool.call_completed"
    );
  }, [events]);

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
    isActive,
    shouldPoll,
    supportsReasoning,
    reasoningEffort,
    setReasoningEffort,
    getReasoningEffortName,
    defaultEffortName,
    isWaitingForResponse,
    setIsWaitingForResponse,
    sendMessage,
    getMessageText,
    getToolCalls,
  };

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}
