// Session and Message hooks (M2)
"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createSession,
  deleteSession,
  forkSession,
  getSession,
  getSessionResolvedModel,
  getSessionContextReport,
  getSessionFacets,
  getSessionStats,
  listSessions,
  updateSession,
  sendUserMessage,
  pinSession,
  unpinSession,
  archiveSession,
  unarchiveSession,
} from "@/lib/api/sessions";
import { DEFAULT_EXCLUDED_EVENTS, getSseUrl, listEventsPaginated } from "@/lib/api/events";
import { queryKeys } from "@/lib/query-keys";
import type {
  CreateSessionRequest,
  ForkSessionRequest,
  UpdateSessionRequest,
  Controls,
  Event,
  PaginationParams,
} from "@/lib/api/types";
import {
  SESSIONS_PAGE_SIZE,
  serializeSessionFilters,
  toQueryParams,
  type SessionFilters,
} from "@/lib/session-filters";
import { useOrg } from "@/providers/org-provider";
import { useLocale } from "@/providers/locale-provider";
import { createReconnectTracker } from "@/lib/sse-reconnect";
import { createEventStream, type EventStreamLike } from "@/lib/event-stream";
import { useResourceOrgFallback } from "./use-resource-org-fallback";

/**
 * Fetch paginated sessions for an organization.
 * Optionally filter by agentId.
 * Returns { data, total, offset, limit } for pagination controls.
 */
export function useSessions(
  agentId?: string,
  params?: PaginationParams,
  options: { enabled?: boolean } = {},
) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessions.list(org, agentId, params?.offset ?? 0, params?.limit ?? 20),
    queryFn: () => listSessions({ ...params, agentId }),
    enabled: !!org && (options.enabled ?? true),
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/**
 * The filtered sessions list behind the operational Sessions page (EVE-853).
 *
 * Pairs with {@link useSessionFacets}: both take the same `SessionFilters`, so
 * the rows and the counts beside them are always the same population.
 */
export function useFilteredSessions(filters: SessionFilters, options: { enabled?: boolean } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const serialized = serializeSessionFilters(filters);
  const params = toQueryParams(filters);
  const offset = filters.page * SESSIONS_PAGE_SIZE;

  const query = useQuery({
    queryKey: queryKeys.sessions.filtered(org, serialized, offset, SESSIONS_PAGE_SIZE),
    queryFn: () => listSessions({ ...params, offset, limit: SESSIONS_PAGE_SIZE }),
    enabled: !!org && (options.enabled ?? true),
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/**
 * Facet counts and masthead metrics for a filter set.
 *
 * Deliberately keyed on the filters minus pagination: paging through a result
 * set does not change what is counted, so turning the page must not refetch or
 * flicker the rail.
 */
export function useSessionFacets(filters: SessionFilters, options: { enabled?: boolean } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const serialized = serializeSessionFilters({ ...filters, page: 0 });
  const params = toQueryParams(filters);

  const query = useQuery({
    queryKey: queryKeys.sessions.facets(org, serialized),
    queryFn: () => getSessionFacets(params),
    enabled: !!org && (options.enabled ?? true),
  });

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

  // Cross-org fallback: auto-switch to the owning org when the user follows a
  // direct link to a session in another org they are a member of.
  // See knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
  const fallback = useResourceOrgFallback({
    resourceId: sessionId,
    error: query.error,
    isLoading: orgLoading || query.isLoading,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading || fallback.isCheckingOtherOrgs,
  };
}

export function useSessionContextReport(sessionId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessions.contextReport(org, sessionId),
    queryFn: () => getSessionContextReport(sessionId!),
    enabled: !!org && !!sessionId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useSessionResolvedModel(sessionId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessions.resolvedModel(org, sessionId),
    queryFn: () => getSessionResolvedModel(sessionId!),
    enabled: !!org && !!sessionId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateSession() {
  const queryClient = useQueryClient();
  const { backendLocale } = useLocale();

  return useMutation({
    mutationFn: ({ request }: { request: CreateSessionRequest }) =>
      createSession({
        ...request,
        locale: request.locale ?? backendLocale,
        // Auto-declare the hints for the interactive cards the Chat UI can
        // render inline: connection setup, and consent for a URL an MCP server
        // asks the user to open. Each hint is what lets the backend hold the
        // turn for that card instead of talking past it.
        hints: { setup_connection: true, url_elicitation: true, ...request.hints },
      }),
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

/**
 * Fork a session into a new one seeded with the parent's context.
 *
 * The session detail page is a read-only recording (EVE-854); forking is the
 * escape hatch that turns a recording back into something you can talk to.
 */
export function useForkSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ sessionId, request }: { sessionId: string; request?: ForkSessionRequest }) =>
      forkSession(sessionId, request),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
      if (session.agent_id) {
        queryClient.invalidateQueries({
          queryKey: queryKeys.sessions.byAgent(session.agent_id),
        });
      }
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

export function useArchiveSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string }) => archiveSession(sessionId),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions.all() });
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessions.detail(org, sessionId),
      });
    },
  });
}

export function useUnarchiveSession() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string }) => unarchiveSession(sessionId),
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
      addressedParticipantId,
    }: {
      sessionId: string;
      content: string;
      controls?: Controls;
      addressedParticipantId?: string | null;
    }) => sendUserMessage(sessionId, content, controls, addressedParticipantId),
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
  "output.message.replaced",
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
  "tool.progress",
  "tool.completed",
  "tool.output.delta",
  "tool.call_requested",
  "llm.generation",
  "session.started",
  "session.activated",
  "session.idled",
  "reason.thinking.started",
  "reason.thinking.delta",
  "reason.thinking.completed",
  "reason.item",
  "context.compacted",
  "session.model.changed",
];

/** Default page size for paginated event loading */
const EVENT_PAGE_SIZE = 200;

/**
 * Maximum number of events kept in memory. When exceeded, oldest events are
 * trimmed to stay within budget. This prevents unbounded memory growth during
 * long-running SSE sessions.
 */
const MAX_EVENTS_IN_MEMORY = 5_000;

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
  const eventSourceRef = useRef<EventStreamLike | null>(null);
  const lastEventIdRef = useRef<string | null>(null);
  const reconnectRef = useRef(createReconnectTracker());
  const isEnabled = options?.enabled !== false;

  // Track events by ID to avoid duplicates
  const eventIdsRef = useRef<Set<string>>(new Set());

  // Bound the dedup set to the events still in memory. Doing this in an effect
  // (rather than inside the setEvents updater that trims the buffer) keeps that
  // updater pure. The size guard makes this a no-op for normal appends and only
  // rebuilds the set right after the buffer has been trimmed.
  useEffect(() => {
    if (eventIdsRef.current.size > events.length) {
      eventIdsRef.current = new Set(events.map((e) => e.id));
    }
  }, [events]);

  // Track whether initial REST fetch has completed
  const [restLoaded, setRestLoaded] = useState(false);

  // Dedup in-flight REST fetches for the same (org, sessionId) pair (EVE-159)
  const fetchingKeyRef = useRef<string | null>(null);

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
    fetchingKeyRef.current = null;
    setTotalNonDeltaCount(undefined);
    oldestLoadedSequenceRef.current = undefined;
    setHasMore(false);
    setLoadingOlder(false);
    cleanup();
  }, [org, sessionId, cleanup]);

  // Step 1: REST batch load last N events (excluding deltas for old messages)
  useEffect(() => {
    if (!org || !sessionId || !isEnabled) return;

    // Skip if already fetching for the same (org, sessionId) pair (EVE-159)
    const key = `${org}:${sessionId}`;
    if (fetchingKeyRef.current === key) return;
    fetchingKeyRef.current = key;

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
      // Allow re-fetch if effect re-runs after cleanup
      if (fetchingKeyRef.current === key) {
        fetchingKeyRef.current = null;
      }
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

      // No event id yet means the REST snapshot was empty (a fresh session) or
      // failed: resume from sequence 0 so the server replays everything written
      // before this subscription instead of starting live and dropping it.
      const sseUrl = getSseUrl(
        sessionId,
        lastEventIdRef.current ? { sinceId: lastEventIdRef.current } : { afterSequence: 0 },
      );
      const eventSource = createEventStream(sseUrl, { withCredentials: true });
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
            // Keep this updater pure: React may invoke a state updater more than
            // once for a single update (StrictMode, concurrent replays). The
            // dedup set is reconciled to the in-memory window by the effect
            // declared earlier in this hook, not mutated here.
            setEvents((prev) => {
              const next = [...prev, event];
              return next.length > MAX_EVENTS_IN_MEMORY
                ? next.slice(next.length - MAX_EVENTS_IN_MEMORY)
                : next;
            });
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
