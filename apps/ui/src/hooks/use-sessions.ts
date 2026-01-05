// Session and Message hooks (M2)

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createSession,
  deleteSession,
  getSession,
  listSessions,
  updateSession,
  sendUserMessage,
  listEvents,
} from "@/lib/api/sessions";
import { getSseUrl } from "@/lib/api/events";
import type { CreateSessionRequest, UpdateSessionRequest, Controls, Event } from "@/lib/api/types";

export function useSessions(agentId: string | undefined) {
  return useQuery({
    queryKey: ["sessions", agentId],
    queryFn: () => listSessions(agentId!),
    enabled: !!agentId,
  });
}

export function useSession(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  return useQuery({
    queryKey: ["session", agentId, sessionId],
    queryFn: () => getSession(agentId!, sessionId!),
    enabled: !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}

export function useCreateSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      request,
    }: {
      agentId: string;
      request?: CreateSessionRequest;
    }) => createSession(agentId, request),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", agentId] });
    },
  });
}

export function useUpdateSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: UpdateSessionRequest;
    }) => updateSession(agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", agentId] });
      queryClient.invalidateQueries({
        queryKey: ["session", agentId, sessionId],
      });
    },
  });
}

export function useDeleteSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
    }: {
      agentId: string;
      sessionId: string;
    }) => deleteSession(agentId, sessionId),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: ["sessions", agentId] });
    },
  });
}

export function useSendMessage() {
  const queryClient = useQueryClient();

  return useMutation({
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
    }) => sendUserMessage(agentId, sessionId, content, controls),
    onSuccess: (_, { agentId, sessionId }) => {
      // Invalidate events query to refresh the message list
      queryClient.invalidateQueries({
        queryKey: ["events", agentId, sessionId],
      });
    },
  });
}

// ============================================
// Events hook - uses SSE for real-time updates
// ============================================

/**
 * Fetch events for a session using SSE (Server-Sent Events)
 *
 * Uses SSE for real-time streaming with since_id for incremental updates.
 * Falls back to initial fetch + SSE reconnection for reliability.
 * The enabled option controls whether to connect to SSE (useful for inactive sessions).
 */
export function useEvents(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { enabled?: boolean }
) {
  const [events, setEvents] = useState<Event[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const lastEventIdRef = useRef<string | null>(null);
  const isEnabled = options?.enabled !== false;

  // Track events by ID to avoid duplicates
  const eventIdsRef = useRef<Set<string>>(new Set());

  // Cleanup function
  const cleanup = useCallback(() => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
  }, []);

  // Reset state when session changes
  useEffect(() => {
    setEvents([]);
    setIsLoading(true);
    setError(null);
    lastEventIdRef.current = null;
    eventIdsRef.current.clear();
  }, [agentId, sessionId]);

  // SSE connection
  useEffect(() => {
    if (!agentId || !sessionId || !isEnabled) {
      cleanup();
      return;
    }

    const connectSSE = () => {
      // Close existing connection
      cleanup();

      const sseUrl = getSseUrl(agentId, sessionId, lastEventIdRef.current ?? undefined);
      const eventSource = new EventSource(sseUrl, { withCredentials: true });
      eventSourceRef.current = eventSource;

      // Listen for "connected" event to know SSE stream is ready
      // This is sent immediately by the server when connection is established
      eventSource.addEventListener("connected", () => {
        setIsLoading(false);
        setError(null);
      });

      // Fallback: onopen may fire, but "connected" event is more reliable
      eventSource.onopen = () => {
        setError(null);
      };

      // Listen for typed events (the backend sends event type as SSE event name)
      const eventTypes = [
        "message.user",
        "message.agent",
        "turn.started",
        "turn.completed",
        "turn.failed",
        "input.received",
        "reason.started",
        "reason.completed",
        "act.started",
        "act.completed",
        "tool.call_started",
        "tool.call_completed",
        "llm.generation",
        "session.started",
        "session.activated",
        "session.idled",
      ];

      for (const eventType of eventTypes) {
        eventSource.addEventListener(eventType, (messageEvent) => {
          try {
            const event: Event = JSON.parse(messageEvent.data);

            // Skip if we already have this event
            if (eventIdsRef.current.has(event.id)) {
              return;
            }

            eventIdsRef.current.add(event.id);
            lastEventIdRef.current = event.id;
            setEvents((prev) => [...prev, event]);
          } catch (e) {
            console.error("Failed to parse SSE event:", e);
          }
        });
      }

      eventSource.onerror = () => {
        // SSE will auto-reconnect, but we track the error state
        setError(new Error("SSE connection error"));
        // Reconnect after a delay if the connection was lost
        cleanup();
        setTimeout(() => {
          if (isEnabled) {
            connectSSE();
          }
        }, 2000);
      };
    };

    connectSSE();

    return cleanup;
  }, [agentId, sessionId, isEnabled, cleanup]);

  return {
    data: events,
    isLoading,
    error,
  };
}

/**
 * Fetch events for a session using polling (legacy, for comparison)
 * Returns raw Event[] for direct rendering in the UI
 */
export function useEventsPolling(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  return useQuery({
    queryKey: ["events", agentId, sessionId],
    queryFn: () => listEvents(agentId!, sessionId!),
    enabled: !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}
