// SSE Events hook for real-time streaming
// Events follow the standard event protocol: { id, type, ts, context, data }
// Event types: message.user, message.agent, message.tool_call, message.tool_result,
//              turn.started, turn.completed, turn.failed,
//              input.received, reason.started, reason.completed,
//              act.started, act.completed, tool.call_started, tool.call_completed,
//              session.started

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
  // New: raw events for debugging/display
  events: SSEEvent[];
}

// Standard event schema
interface SSEEvent {
  id: string;
  type: string;
  ts: string;
  context: {
    session_id: string;
    turn_id?: string;
    input_message_id?: string;
    exec_id?: string;
  };
  data: Record<string, unknown>;
  sequence?: number;
}

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9000";

export function useSSEEvents({
  agentId,
  sessionId,
  enabled = true,
}: UseSSEEventsOptions): UseSSEEventsReturn {
  const [messages, setMessages] = useState<AggregatedMessage[]>([]);
  const [toolCalls, setToolCalls] = useState<AggregatedToolCall[]>([]);
  const [events, setEvents] = useState<SSEEvent[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  const eventSourceRef = useRef<EventSource | null>(null);

  const handleEvent = useCallback((event: MessageEvent) => {
    try {
      const sseEvent: SSEEvent = JSON.parse(event.data);

      // Store all events for debugging/display
      setEvents((prev) => [...prev, sseEvent]);

      const { data } = sseEvent;

      switch (event.type) {
        // =====================================================================
        // Message Events
        // =====================================================================
        case "message.user":
          setMessages((prev) => [
            ...prev,
            {
              id: (data.message_id as string) || sseEvent.id,
              role: "user",
              content: extractTextContent(data.content),
              isComplete: true,
            },
          ]);
          break;

        case "message.agent":
          setMessages((prev) => [
            ...prev,
            {
              id: (data.message_id as string) || sseEvent.id,
              role: "assistant",
              content: extractTextContent(data.content),
              isComplete: true,
            },
          ]);
          break;

        case "message.tool_call":
          // Tool calls requested by assistant
          if (Array.isArray(data.tool_calls)) {
            for (const tc of data.tool_calls) {
              setToolCalls((prev) => [
                ...prev,
                {
                  id: tc.id as string,
                  name: tc.name as string,
                  arguments: (tc.arguments as Record<string, unknown>) || {},
                  isComplete: false,
                },
              ]);
            }
          }
          break;

        case "message.tool_result":
          // Tool result came back
          setToolCalls((prev) =>
            prev.map((tc) =>
              tc.id === (data.tool_call_id as string)
                ? {
                    ...tc,
                    isComplete: true,
                    result: data.content,
                    error: data.is_error ? "Tool returned error" : undefined,
                  }
                : tc
            )
          );
          break;

        // =====================================================================
        // Turn Lifecycle Events (for observability)
        // =====================================================================
        case "turn.started":
        case "turn.completed":
        case "turn.failed":
          // Turn lifecycle events - stored but not displayed directly
          break;

        // =====================================================================
        // Atom Lifecycle Events (for observability)
        // =====================================================================
        case "input.received":
        case "reason.started":
        case "reason.completed":
        case "act.started":
        case "act.completed":
          // These are observability events - stored but not displayed directly
          break;

        case "tool.call_started":
          // Individual tool call started
          {
            const toolCall = data.tool_call as {
              id: string;
              name: string;
              arguments: Record<string, unknown>;
            };
            if (toolCall) {
              setToolCalls((prev) => {
                // Only add if not already present
                const existing = prev.find((tc) => tc.id === toolCall.id);
                if (existing) return prev;
                return [
                  ...prev,
                  {
                    id: toolCall.id,
                    name: toolCall.name,
                    arguments: toolCall.arguments || {},
                    isComplete: false,
                  },
                ];
              });
            }
          }
          break;

        case "tool.call_completed":
          // Individual tool call completed
          setToolCalls((prev) =>
            prev.map((tc) =>
              tc.id === (data.tool_call_id as string)
                ? {
                    ...tc,
                    isComplete: true,
                    error: data.error as string | undefined,
                  }
                : tc
            )
          );
          break;

        // =====================================================================
        // Session Events
        // =====================================================================
        case "session.started":
          // Session lifecycle event
          break;

        default:
          // Unknown event type - log for debugging
          console.debug("Unknown SSE event type:", event.type, sseEvent);
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

    const url = `${API_BASE}/v1/agents/${agentId}/sessions/${sessionId}/sse`;
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

    // Listen to all event types from the event protocol
    const eventTypes = [
      // Message events
      "message.user",
      "message.agent",
      "message.tool_call",
      "message.tool_result",
      // Turn lifecycle events
      "turn.started",
      "turn.completed",
      "turn.failed",
      // Atom lifecycle events
      "input.received",
      "reason.started",
      "reason.completed",
      "act.started",
      "act.completed",
      "tool.call_started",
      "tool.call_completed",
      // Session events
      "session.started",
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
    events,
  };
}

// Helper to extract text from content array
function extractTextContent(content: unknown): string {
  if (!Array.isArray(content)) return "";

  return content
    .filter((part: unknown) => {
      return (
        typeof part === "object" &&
        part !== null &&
        "type" in part &&
        part.type === "text"
      );
    })
    .map((part: unknown) => {
      const p = part as { text?: string };
      return p.text || "";
    })
    .join("");
}
