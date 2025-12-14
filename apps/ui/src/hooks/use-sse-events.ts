"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import { getApiBaseUrl } from "@/lib/api/client";
import type { AgUiEvent } from "@/lib/api/types";

interface UseSSEEventsOptions {
  runId: string;
  enabled?: boolean;
  onEvent?: (event: AgUiEvent) => void;
}

interface UseSSEEventsReturn {
  events: AgUiEvent[];
  isConnected: boolean;
  error: Error | null;
  disconnect: () => void;
}

export function useSSEEvents({
  runId,
  enabled = true,
  onEvent,
}: UseSSEEventsOptions): UseSSEEventsReturn {
  const [events, setEvents] = useState<AgUiEvent[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  const connect = useCallback(() => {
    if (!enabled || !runId) return;

    const url = `${getApiBaseUrl()}/v1/runs/${runId}/events`;

    const eventSource = new EventSource(url);
    eventSourceRef.current = eventSource;

    eventSource.onopen = () => {
      setIsConnected(true);
      setError(null);
    };

    eventSource.onerror = () => {
      setIsConnected(false);
      setError(new Error("SSE connection failed"));
    };

    // Handle specific event types
    const eventTypes = [
      "RUN_STARTED",
      "RUN_FINISHED",
      "RUN_ERROR",
      "TEXT_MESSAGE_START",
      "TEXT_MESSAGE_CHUNK",
      "TEXT_MESSAGE_END",
      "TOOL_CALL_START",
      "TOOL_CALL_RESULT",
      "STEP_STARTED",
      "STEP_FINISHED",
      "CUSTOM",
    ];

    eventTypes.forEach((type) => {
      eventSource.addEventListener(type, (e: MessageEvent) => {
        try {
          const data = JSON.parse(e.data);
          const event: AgUiEvent = { type: type as AgUiEvent["type"], ...data };

          setEvents((prev) => [...prev, event]);
          onEvent?.(event);
        } catch (err) {
          console.error("Failed to parse SSE event:", err);
        }
      });
    });

    return eventSource;
  }, [runId, enabled, onEvent]);

  useEffect(() => {
    const eventSource = connect();

    return () => {
      eventSource?.close();
    };
  }, [connect]);

  const disconnect = useCallback(() => {
    eventSourceRef.current?.close();
    setIsConnected(false);
  }, []);

  return {
    events,
    isConnected,
    error,
    disconnect,
  };
}

// Utility to aggregate text messages from events
export interface AggregatedMessage {
  id: string;
  role: string;
  content: string;
  isComplete: boolean;
}

export function aggregateTextMessages(events: AgUiEvent[]): AggregatedMessage[] {
  const messages = new Map<string, AggregatedMessage>();

  for (const event of events) {
    if (event.type === "TEXT_MESSAGE_START") {
      messages.set(event.message_id, {
        id: event.message_id,
        role: event.role,
        content: "",
        isComplete: false,
      });
    } else if (event.type === "TEXT_MESSAGE_CHUNK") {
      const msg = messages.get(event.message_id);
      if (msg) {
        msg.content += event.chunk;
      }
    } else if (event.type === "TEXT_MESSAGE_END") {
      const msg = messages.get(event.message_id);
      if (msg) {
        msg.isComplete = true;
      }
    }
  }

  return Array.from(messages.values());
}

// Utility to get tool calls from events
export interface AggregatedToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
  result: Record<string, unknown> | null;
  error: string | null;
  isComplete: boolean;
}

export function aggregateToolCalls(events: AgUiEvent[]): AggregatedToolCall[] {
  const toolCalls = new Map<string, AggregatedToolCall>();

  for (const event of events) {
    if (event.type === "TOOL_CALL_START") {
      toolCalls.set(event.tool_call_id, {
        id: event.tool_call_id,
        name: event.tool_name,
        arguments: event.arguments,
        result: null,
        error: null,
        isComplete: false,
      });
    } else if (event.type === "TOOL_CALL_RESULT") {
      const tc = toolCalls.get(event.tool_call_id);
      if (tc) {
        tc.result = event.result;
        tc.error = event.error;
        tc.isComplete = true;
      }
    }
  }

  return Array.from(toolCalls.values());
}
