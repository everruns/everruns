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
import type { CreateSessionRequest, UpdateSessionRequest, Controls, Event, PaginationParams } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

/**
 * Fetch paginated sessions for an agent.
 * Returns { data, total, offset, limit } for pagination controls.
 */
export function useSessions(
  agentId: string | undefined,
  params?: PaginationParams
) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.sessions.list(agentId!), org, params?.offset ?? 0, params?.limit ?? 20],
    queryFn: () => listSessions(org!, agentId!, params),
    enabled: !!org && !!agentId,
  });
}

export function useSession(
  agentId: string | undefined,
  sessionId: string | undefined,
  options?: { refetchInterval?: number | false }
) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.sessions.detail(agentId!, sessionId!), org],
    queryFn: () => getSession(org!, agentId!, sessionId!),
    enabled: !!org && !!agentId && !!sessionId,
    refetchInterval: options?.refetchInterval,
  });
}

export function useCreateSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      request,
    }: {
      agentId: string;
      request?: CreateSessionRequest;
    }) => createSession(org!, agentId, request),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.list(agentId) });
    },
  });
}

export function useUpdateSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: UpdateSessionRequest;
    }) => updateSession(org!, agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.list(agentId) });
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessions.detail(agentId, sessionId),
      });
    },
  });
}

export function useDeleteSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
    }: {
      agentId: string;
      sessionId: string;
    }) => deleteSession(org!, agentId, sessionId),
    onSuccess: (_, { agentId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.list(agentId) });
    },
  });
}

export function useSendMessage() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

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
    }) => sendUserMessage(org!, agentId, sessionId, content, controls),
    onSuccess: (_, { agentId, sessionId }) => {
      // Invalidate events query to refresh the message list
      queryClient.invalidateQueries({
        queryKey: queryKeys.events.list(agentId, sessionId),
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
  }, [org, agentId, sessionId]);

  // SSE connection
  useEffect(() => {
    if (!org || !agentId || !sessionId || !isEnabled) {
      cleanup();
      return;
    }

    const connectSSE = () => {
      // Close existing connection
      cleanup();

      const sseUrl = getSseUrl(org, agentId, sessionId, lastEventIdRef.current ?? undefined);
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
  }, [org, agentId, sessionId, isEnabled, cleanup]);

  return {
    data: events,
    isLoading,
    error,
  };
}
