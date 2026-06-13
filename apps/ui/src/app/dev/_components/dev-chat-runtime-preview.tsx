"use client";

import { useMemo, useState } from "react";
import { ChatPanel } from "@/components/chat/chat-panel";
import { type SessionContextValue } from "@/app/(main)/sessions/[sessionId]/session-context";
import type { Agent, Event, ModelWithProvider, Message, Session } from "@/lib/api/types";
import { getTextFromContent, getToolCallsFromContent } from "@/lib/api/types";
import { getLocalizedOutputMessageText } from "@/lib/runtime-errors";
import { getDevChatFixture } from "@/app/dev/_fixtures/chat-runtime-fixtures";
import { useLocale } from "@/providers/locale-provider";
import { DevSessionRuntimeFrame } from "@/app/dev/_components/dev-session-runtime-frame";

function createNoopCancelMutation(): SessionContextValue["cancelCurrentTurn"] {
  return {
    mutate: () => undefined,
    isPending: false,
  } as SessionContextValue["cancelCurrentTurn"];
}

export function DevChatRuntimeScene({
  scenario,
}: {
  scenario: "tool-activity" | "chat-components";
}) {
  const fixture = useMemo(() => getDevChatFixture(scenario), [scenario]);
  const { locale } = useLocale();
  const models = useMemo<ModelWithProvider[]>(
    () =>
      fixture.models.map((model) => ({
        ...model,
        provider_name: model.provider_id === "provider-openai" ? "OpenAI" : "OpenRouter",
        provider_type: "openai",
        healthy: true,
        profile: {
          name: model.display_name,
          family: "kimi",
          attachment: true,
          reasoning: true,
          temperature: true,
          tool_call: true,
          structured_output: true,
          open_weights: false,
          reasoning_effort: {
            default: "medium",
            values: [
              { value: "low", name: "Low" },
              { value: "medium", name: "Medium" },
              { value: "high", name: "High" },
            ],
          },
        },
      })),
    [fixture.models],
  );
  const [events, setEvents] = useState<Event[]>(fixture.events);
  const [reasoningEffort, setReasoningEffort] = useState<SessionContextValue["reasoningEffort"]>(
    fixture.initialReasoningEffort as SessionContextValue["reasoningEffort"],
  );
  const [isWaitingForResponse, setIsWaitingForResponse] = useState(false);

  const agent: Agent = {
    id: "agent_dev_preview",
    name: "platform-chat",
    display_name: "Platform Chat",
    description: "Fixture agent for runtime chat previews.",
    system_prompt: "You are a precise coding assistant.",
    default_model_id: models[0].id,
    tags: [],
    capabilities: [],
    initial_files: [],
    status: "active",
    created_at: "2026-03-07T21:00:00Z",
    updated_at: "2026-03-07T21:00:00Z",
    archived_at: null,
    deleted_at: null,
  };

  const session: Session = {
    id: fixture.sessionId,
    organization_id: "org_dev_preview",
    harness_id: "harness_platform",
    agent_id: agent.id,
    owner_principal_id: "user_dev_preview",
    title:
      scenario === "tool-activity" ? "Daytona sandbox tool activity" : "Runtime chat components",
    preview: null,
    output_preview: null,
    tags: [],
    model_id: models[0].id,
    status: scenario === "tool-activity" ? "active" : "idle",
    created_at: "2026-03-07T21:20:00Z",
    updated_at: "2026-03-07T21:22:00Z",
    started_at: "2026-03-07T21:20:00Z",
    finished_at: null,
    active_schedule_count: scenario === "tool-activity" ? 1 : 0,
    features: ["file_system", "leased_resources", "schedules"],
  };

  const sendMessage = useMemo(() => {
    const mutateAsync = async ({
      sessionId,
      content,
    }: {
      sessionId: string;
      content: string;
    }): Promise<Message> => {
      const ts = new Date().toISOString();
      const message: Message = {
        id: `dev-msg-${Date.now()}`,
        session_id: sessionId,
        sequence: events.length + 10,
        role: "user",
        content: [{ type: "text", text: content }],
        tool_call_id: null,
        created_at: ts,
      };

      setEvents((current) => [
        ...current,
        {
          id: `dev-event-${Date.now()}`,
          type: "input.message",
          ts,
          session_id: sessionId,
          context: {},
          data: { message },
        },
      ]);

      return message;
    };

    return {
      mutateAsync,
      isPending: false,
    } as SessionContextValue["sendMessage"];
  }, [events.length]);

  const contextValue: SessionContextValue = {
    agentId: "agent_dev_preview",
    sessionId: fixture.sessionId,
    agent,
    session,
    events,
    llmModel: models[0],
    chatEvents: events,
    toolResultsMap: fixture.toolResultsMap,
    toolProgressMap: fixture.toolProgressMap,
    toolOutputMap: fixture.toolOutputMap,
    sessionLoading: false,
    eventsLoading: false,
    effectiveStatus: session.status,
    liveUsage: { input_tokens: 1842, output_tokens: 326 },
    isActive: scenario === "tool-activity",
    shouldPoll: false,
    supportsReasoning: true,
    reasoningEffort,
    setReasoningEffort,
    getReasoningEffortName: (value: string) =>
      value === "medium" ? "Medium" : value === "high" ? "High" : value === "low" ? "Low" : value,
    defaultEffortName: "Medium",
    isWaitingForResponse,
    setIsWaitingForResponse,
    isThinking: scenario === "chat-components",
    streamingText:
      scenario === "chat-components"
        ? "Applying the runtime chat surface directly to this dev page."
        : null,
    streamingTurnId: scenario === "chat-components" ? "turn-dev-streaming" : null,
    streamingIteration: scenario === "chat-components" ? 2 : null,
    sendMessage,
    cancelCurrentTurn: createNoopCancelMutation(),
    hasMoreEvents: false,
    loadingOlderEvents: false,
    loadOlderEvents: async () => undefined,
    totalNonDeltaCount: events.length,
    getMessageText: (data) =>
      data.message?.role === "agent"
        ? getLocalizedOutputMessageText(locale, data)
        : getTextFromContent(data.message?.content ?? []),
    getToolCalls: (data) => getToolCallsFromContent(data.message?.content ?? []),
  };

  return (
    <DevSessionRuntimeFrame contextValue={contextValue}>
      <ChatPanel />
    </DevSessionRuntimeFrame>
  );
}
