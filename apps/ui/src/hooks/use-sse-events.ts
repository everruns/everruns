// SSE Events hook for real-time streaming (M2)

import { useEffect, useRef, useState, useCallback } from "react";

export interface AggregatedMessage {
  id: string;
  role: string;
  content: string;
  isComplete: boolean;
}

export interface AggregatedToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
  isComplete: boolean;
  result?: unknown;
  error?: string;
}

interface UseSSEEventsOptions {
  agentId: string;
  sessionId: string;
  enabled?: boolean;
}

interface UseSSEEventsReturn {
  messages: AggregatedMessage[];
  toolCalls: AggregatedToolCall[];
  isConnected: boolean;
  error: Error | null;
}

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9000";

export function useSSEEvents({
  agentId,
  sessionId,
  enabled = true,
}: UseSSEEventsOptions): UseSSEEventsReturn {
  const [messages, setMessages] = useState<AggregatedMessage[]>([]);
  const [toolCalls, setToolCalls] = useState<AggregatedToolCall[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  const eventSourceRef = useRef<EventSource | null>(null);

  const handleEvent = useCallback((event: MessageEvent) => {
    try {
      const data = JSON.parse(event.data);

      switch (event.type) {
        case "step.generating":
          // Streaming text delta
          setMessages((prev) => {
            const existing = prev.find((m) => m.id === data.message_id);
            if (existing) {
              return prev.map((m) =>
                m.id === data.message_id
                  ? { ...m, content: m.content + (data.delta || "") }
                  : m
              );
            }
            return [
              ...prev,
              {
                id: data.message_id || `streaming-${Date.now()}`,
                role: "assistant",
                content: data.delta || "",
                isComplete: false,
              },
            ];
          });
          break;

        case "step.generated":
          // Complete the streaming message
          setMessages((prev) =>
            prev.map((m) =>
              m.id === data.message_id ? { ...m, isComplete: true } : m
            )
          );
          break;

        case "tool.started":
          setToolCalls((prev) => [
            ...prev,
            {
              id: data.tool_call_id,
              name: data.name,
              arguments: data.arguments || {},
              isComplete: false,
            },
          ]);
          break;

        case "tool.completed":
          setToolCalls((prev) =>
            prev.map((tc) =>
              tc.id === data.tool_call_id
                ? {
                    ...tc,
                    isComplete: true,
                    result: data.result,
                    error: data.error,
                  }
                : tc
            )
          );
          break;

        case "message.created":
          // A message was persisted - could refresh messages list
          break;

        case "session.completed":
        case "session.failed":
          // Session ended
          break;

        default:
          // Unknown event type
          break;
      }
    } catch (e) {
      console.error("Failed to parse SSE event:", e);
    }
  }, []);

  useEffect(() => {
    if (!enabled || !agentId || !sessionId) {
      return;
    }

    const url = `${API_BASE}/v1/agents/${agentId}/sessions/${sessionId}/events`;
    const eventSource = new EventSource(url);
    eventSourceRef.current = eventSource;

    eventSource.onopen = () => {
      setIsConnected(true);
      setError(null);
    };

    eventSource.onerror = (e) => {
      setIsConnected(false);
      setError(new Error("SSE connection error"));
      console.error("SSE error:", e);
    };

    // Listen to specific event types
    const eventTypes = [
      "step.started",
      "step.generating",
      "step.generated",
      "step.error",
      "message.created",
      "message.delta",
      "tool.started",
      "tool.completed",
      "session.started",
      "session.completed",
      "session.failed",
    ];

    eventTypes.forEach((type) => {
      eventSource.addEventListener(type, handleEvent);
    });

    return () => {
      eventTypes.forEach((type) => {
        eventSource.removeEventListener(type, handleEvent);
      });
      eventSource.close();
      eventSourceRef.current = null;
    };
  }, [agentId, sessionId, enabled, handleEvent]);

  return {
    messages,
    toolCalls,
    isConnected,
    error,
  };
}
