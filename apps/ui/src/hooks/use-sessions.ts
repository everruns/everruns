// Session and Message hooks (M2)
"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createSession,
  deleteSession,
  getSession,
  listSessions,
  updateSession,
  sendUserMessage,
} from "@/lib/api/sessions";
import { getSseUrl } from "@/lib/api/events";
import { queryKeys } from "@/lib/query-keys";
import type {
  CreateSessionRequest,
  UpdateSessionRequest,
  Controls,
  Event,
  PaginationParams,
} from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

/**
 * Fetch paginated sessions for an organization.
 * Optionally filter by agentId.
 * Returns { data, total, offset, limit } for pagination controls.
 */
export function useSessions(agentId?: string, params?: PaginationParams) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessions.list(
      org,
      agentId,
      params?.offset ?? 0,
      params?.limit ?? 20,
    ),
    queryFn: () => listSessions({ ...params, agentId }),
    enabled: !!org,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useSession(
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false },
) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessions.detail(org, sessionId!),
    queryFn: () => getSession(sessionId!),
    enabled: !!org && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ request }: { request: CreateSessionRequest }) =>
      createSession(request),
    onSuccess: (_, { request }) => {
      // Invalidate sessions list - both all sessions and agent-specific
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
      if (request.agent_id) {
        queryClient.invalidateQueries({
          queryKey: queryKeys.sessions.byAgent(request.agent_id),
        });
      }
    },
  });
}

export function useUpdateSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      sessionId,
      request,
    }: {
      sessionId: string;
      request: UpdateSessionRequest;
    }) => updateSession(sessionId, request),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessions.detail(org, sessionId),
      });
    },
  });
}

export function useDeleteSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string }) =>
      deleteSession(sessionId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
    },
  });
}

export function useSendMessage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      sessionId,
      content,
      controls,
    }: {
      sessionId: string;
      content: string;
      controls?: Controls;
    }) => sendUserMessage(sessionId, content, controls),
    onSuccess: (_, { sessionId }) => {
      // Invalidate events query to refresh the message list
      queryClient.invalidateQueries({
        queryKey: queryKeys.events.list(sessionId),
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
  sessionId: string | undefined,
  options?: { enabled?: boolean },
) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

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
  }, [org, sessionId]);

  // SSE connection
  useEffect(() => {
    if (!org || !sessionId || !isEnabled) {
      cleanup();
      return;
    }

    const connectSSE = () => {
      // Close existing connection
      cleanup();

      const sseUrl = getSseUrl(sessionId, lastEventIdRef.current ?? undefined);
      const eventSource = new EventSource(sseUrl, { withCredentials: true });
      eventSourceRef.current = eventSource;

      // Listen for "connected" event to know SSE stream is ready
      // This is sent immediately by the server when connection is established
      eventSource.addEventListener("connected", () => {
        setIsLoading(false);
        setError(null);
      });

      // Listen for "disconnecting" event for graceful connection cycling
      // Server sends this before closing to allow immediate reconnect with since_id
      eventSource.addEventListener("disconnecting", (messageEvent) => {
        try {
          const data = JSON.parse(messageEvent.data);
          const retryMs = data.retry_ms ?? 100;
          console.debug("SSE disconnecting event received, reconnecting in", retryMs, "ms");
          cleanup();
          setTimeout(() => {
            if (isEnabled) {
              connectSSE();
            }
          }, retryMs);
        } catch {
          // Fallback: reconnect immediately
          cleanup();
          if (isEnabled) {
            connectSSE();
          }
        }
      });

      // Fallback: onopen may fire, but "connected" event is more reliable
      eventSource.onopen = () => {
        setError(null);
      };

      // Listen for typed events (the backend sends event type as SSE event name)
      const eventTypes = [
        "input.message",
        "output.message.started",
        "output.message.delta",
        "output.message.completed",
        "turn.started",
        "turn.completed",
        "turn.failed",
        "turn.cancelled",
        "reason.started",
        "reason.completed",
        "act.started",
        "act.completed",
        "tool.started",
        "tool.completed",
        "llm.generation",
        "session.started",
        "session.activated",
        "session.idled",
        // Streaming events for real-time text updates
        "reason.thinking.started",
        "reason.thinking.delta",
        "reason.thinking.completed",
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
  }, [org, sessionId, isEnabled, cleanup]);

  return {
    data: events,
    isLoading,
    error,
  };
}
