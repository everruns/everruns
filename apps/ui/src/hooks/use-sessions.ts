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
  pinSession,
  unpinSession,
} from "@/lib/api/sessions";
import { getSseUrl, listEvents } from "@/lib/api/events";
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
    queryKey: queryKeys.sessions.list(org, agentId, params?.offset ?? 0, params?.limit ?? 20),
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
    mutationFn: ({ request }: { request: CreateSessionRequest }) => createSession(request),
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
    mutationFn: ({ sessionId, request }: { sessionId: string; request: UpdateSessionRequest }) =>
      updateSession(sessionId, request),
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
    mutationFn: ({ sessionId }: { sessionId: string }) => deleteSession(sessionId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
    },
  });
}

export function usePinSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string }) => pinSession(sessionId),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessions.detail(org, sessionId),
      });
    },
  });
}

export function useUnpinSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string }) => unpinSession(sessionId),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessions.detail(org, sessionId),
      });
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
// Events hook - REST batch load + SSE for live updates
// ============================================

// SSE event types to listen for
const SSE_EVENT_TYPES = [
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
  "reason.thinking.started",
  "reason.thinking.delta",
  "reason.thinking.completed",
];

/**
 * Fetch events for a session using REST + SSE.
 *
 * Strategy: REST first, SSE second.
 * 1. Fetch all existing events via REST (single HTTP response → single setState)
 * 2. Connect SSE with since_id from last REST event for live incremental updates
 *
 * This is critical for large sessions (1000+ turns, 30k+ events) where
 * SSE-only would trigger one state update per event on initial load.
 */
export function useEvents(sessionId: string | undefined, options?: { enabled?: boolean }) {
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

  // Track whether initial REST fetch has completed
  const [restLoaded, setRestLoaded] = useState(false);

  // Cleanup SSE connection
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
    setRestLoaded(false);
    cleanup();
  }, [org, sessionId, cleanup]);

  // Step 1: REST batch load existing events
  useEffect(() => {
    if (!org || !sessionId || !isEnabled) return;

    let cancelled = false;

    async function fetchInitialEvents() {
      try {
        const batch = await listEvents(sessionId!);
        if (cancelled) return;

        if (batch.length > 0) {
          for (const e of batch) {
            eventIdsRef.current.add(e.id);
          }
          lastEventIdRef.current = batch[batch.length - 1].id;
          // Single state update for entire batch
          setEvents(batch);
        }
      } catch (e) {
        if (cancelled) return;
        console.error("Failed to fetch initial events, falling back to SSE-only:", e);
      }

      if (!cancelled) {
        setIsLoading(false);
        setRestLoaded(true);
      }
    }

    fetchInitialEvents();
    return () => {
      cancelled = true;
    };
  }, [org, sessionId, isEnabled]);

  // Step 2: SSE for live updates (starts after REST completes)
  useEffect(() => {
    if (!org || !sessionId || !isEnabled || !restLoaded) {
      return;
    }

    const connectSSE = () => {
      cleanup();

      const sseUrl = getSseUrl(sessionId, lastEventIdRef.current ?? undefined);
      const eventSource = new EventSource(sseUrl, { withCredentials: true });
      eventSourceRef.current = eventSource;

      eventSource.addEventListener("connected", () => {
        setError(null);
      });

      eventSource.addEventListener("disconnecting", (messageEvent) => {
        try {
          const data = JSON.parse(messageEvent.data);
          const retryMs = data.retry_ms ?? 100;
          cleanup();
          setTimeout(() => {
            if (isEnabled) connectSSE();
          }, retryMs);
        } catch {
          cleanup();
          if (isEnabled) connectSSE();
        }
      });

      eventSource.onopen = () => {
        setError(null);
      };

      for (const eventType of SSE_EVENT_TYPES) {
        eventSource.addEventListener(eventType, (messageEvent) => {
          try {
            const event: Event = JSON.parse(messageEvent.data);

            if (eventIdsRef.current.has(event.id)) return;

            eventIdsRef.current.add(event.id);
            lastEventIdRef.current = event.id;
            setEvents((prev) => [...prev, event]);
          } catch (e) {
            console.error("Failed to parse SSE event:", e);
          }
        });
      }

      eventSource.onerror = () => {
        setError(new Error("SSE connection error"));
        cleanup();
        setTimeout(() => {
          if (isEnabled) connectSSE();
        }, 2000);
      };
    };

    connectSSE();

    return cleanup;
  }, [org, sessionId, isEnabled, restLoaded, cleanup]);

  return {
    data: events,
    isLoading,
    error,
  };
}
