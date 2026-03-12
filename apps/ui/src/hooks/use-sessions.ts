// Session and Message hooks (M2)
"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createSession,
  deleteSession,
  getSession,
  getSessionStats,
  listSessions,
  updateSession,
  sendUserMessage,
  pinSession,
  unpinSession,
} from "@/lib/api/sessions";
import { DEFAULT_EXCLUDED_EVENTS, getSseUrl, listEventsPaginated } from "@/lib/api/events";
import { queryKeys } from "@/lib/query-keys";
import type {
  CreateSessionRequest,
  UpdateSessionRequest,
  Controls,
  Event,
  PaginationParams,
} from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";
import { createReconnectTracker } from "@/lib/sse-reconnect";

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

/** Fetch session counts grouped by status for the current organization. */
export function useSessionStats() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessions.stats(org),
    queryFn: () => getSessionStats(),
    enabled: !!org,
  });

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

/** Default page size for paginated event loading */
const EVENT_PAGE_SIZE = 200;

/**
 * Fetch events for a session using paginated REST + SSE.
 *
 * Strategy: REST first (last N events), SSE second.
 * 1. Fetch last 200 non-delta events via REST (single HTTP response → single setState)
 * 2. Connect SSE with since_id from last REST event for live incremental updates
 * 3. When user scrolls up, load older events via REST with before_sequence cursor
 *
 * Pagination uses X-Total-Count header for scroll estimation.
 */
export function useEvents(sessionId: string | undefined, options?: { enabled?: boolean }) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  const [events, setEvents] = useState<Event[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  const [isReconnecting, setIsReconnecting] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);
  const lastEventIdRef = useRef<string | null>(null);
  const reconnectRef = useRef(createReconnectTracker());
  const isEnabled = options?.enabled !== false;

  // Track events by ID to avoid duplicates
  const eventIdsRef = useRef<Set<string>>(new Set());

  // Track whether initial REST fetch has completed
  const [restLoaded, setRestLoaded] = useState(false);

  // Pagination state
  const [totalNonDeltaCount, setTotalNonDeltaCount] = useState<number | undefined>();
  const oldestLoadedSequenceRef = useRef<number | undefined>(undefined);
  const [hasMore, setHasMore] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);

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
    setIsReconnecting(false);
    lastEventIdRef.current = null;
    eventIdsRef.current.clear();
    reconnectRef.current = createReconnectTracker();
    setRestLoaded(false);
    setTotalNonDeltaCount(undefined);
    oldestLoadedSequenceRef.current = undefined;
    setHasMore(false);
    setLoadingOlder(false);
    cleanup();
  }, [org, sessionId, cleanup]);

  // Step 1: REST batch load last N events (excluding deltas for old messages)
  useEffect(() => {
    if (!org || !sessionId || !isEnabled) return;

    let cancelled = false;

    async function fetchInitialEvents() {
      try {
        const result = await listEventsPaginated(sessionId!, {
          limit: EVENT_PAGE_SIZE,
          exclude: DEFAULT_EXCLUDED_EVENTS,
        });
        if (cancelled) return;

        const batch = result.events;
        if (batch.length > 0) {
          for (const e of batch) {
            eventIdsRef.current.add(e.id);
          }
          lastEventIdRef.current = batch[batch.length - 1].id;
          // Track oldest loaded sequence for backward pagination
          const firstSeq = batch[0].sequence;
          if (firstSeq !== undefined) {
            oldestLoadedSequenceRef.current = firstSeq;
          }
          // Single state update for entire batch
          setEvents(batch);
        }

        if (result.totalNonDeltaCount !== undefined) {
          setTotalNonDeltaCount(result.totalNonDeltaCount);
          // There are more events if we loaded fewer than total
          setHasMore(batch.length < result.totalNonDeltaCount);
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

  // Load older events (triggered by scrolling up)
  const loadOlderEvents = useCallback(async () => {
    if (!sessionId || loadingOlder || !hasMore || oldestLoadedSequenceRef.current === undefined) {
      return;
    }

    setLoadingOlder(true);
    try {
      const result = await listEventsPaginated(sessionId, {
        limit: EVENT_PAGE_SIZE,
        before_sequence: oldestLoadedSequenceRef.current,
        exclude: DEFAULT_EXCLUDED_EVENTS,
      });

      const older = result.events;
      if (older.length > 0) {
        for (const e of older) {
          eventIdsRef.current.add(e.id);
        }
        const firstSeq = older[0].sequence;
        if (firstSeq !== undefined) {
          oldestLoadedSequenceRef.current = firstSeq;
        }
        // Prepend older events
        setEvents((prev) => [...older, ...prev]);
        // Check if there are still more
        if (result.totalNonDeltaCount !== undefined) {
          setTotalNonDeltaCount(result.totalNonDeltaCount);
        }
        // No more if we got fewer than requested
        if (older.length < EVENT_PAGE_SIZE) {
          setHasMore(false);
        }
      } else {
        setHasMore(false);
      }
    } catch (e) {
      console.error("Failed to load older events:", e);
    } finally {
      setLoadingOlder(false);
    }
  }, [sessionId, loadingOlder, hasMore]);

  // Step 2: SSE for live updates (starts after REST completes)
  // SSE is unfiltered — picks up deltas for in-flight streaming messages naturally
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
        reconnectRef.current.reset();
        setError(null);
        setIsReconnecting(false);
      });

      eventSource.addEventListener("disconnecting", (messageEvent) => {
        try {
          const data = JSON.parse(messageEvent.data);
          const retryMs = reconnectRef.current.onGraceful(data.retry_ms ?? 100);
          cleanup();
          setIsReconnecting(true);
          setTimeout(() => {
            if (isEnabled) connectSSE();
          }, retryMs);
        } catch {
          cleanup();
          setIsReconnecting(true);
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
        cleanup();
        const delayMs = reconnectRef.current.onError();
        if (delayMs === null) {
          setError(new Error("SSE connection failed after max retries"));
          setIsReconnecting(false);
          return;
        }
        setError(new Error("SSE connection error, reconnecting..."));
        setIsReconnecting(true);
        setTimeout(() => {
          if (isEnabled) connectSSE();
        }, delayMs);
      };
    };

    connectSSE();

    return cleanup;
  }, [org, sessionId, isEnabled, restLoaded, cleanup]);

  return {
    data: events,
    isLoading,
    isReconnecting,
    error,
    // Pagination
    hasMore,
    loadingOlder,
    loadOlderEvents,
    totalNonDeltaCount,
  };
}
