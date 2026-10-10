// Session Trace data: the overview through React Query, and a window of loaded
// turns grown page by page around wherever the reader is.
//
// Decision: loaded turns live in component state rather than an infinite
// query, because the window grows in both directions, jumps to arbitrary
// turns from the minimap, and refreshes only its tail while a session runs.
"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInfiniteQuery, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { searchSessionEvents } from "@/lib/api/events";
import {
  getSessionTrace,
  getSessionTraceStep,
  listSessionTraceRequest,
  listSessionTraceSteps,
  listSessionTraceTurnEvents,
  listSessionTraceTurns,
  type TraceTurnsCursor,
} from "@/lib/api/session-trace";
import type { TraceBatch, TraceGap, TraceStep, TraceTurn } from "@/lib/api/types";
import {
  batchKey,
  fillGap,
  lifecycleEvents,
  mergeTurns,
  stepRowKey,
  type Expansion,
  type LifecycleEvent,
} from "@/components/session/trace/trace-model";
import { useOrg } from "@/providers/org-provider";

const PAGE = 20;
/** Steps fetched per "show more" on a gap. */
const GAP_PAGE = 100;

export const traceKeys = {
  overview: (org: string | undefined, sessionId: string) =>
    ["session-trace", org, sessionId, "overview"] as const,
  step: (org: string | undefined, sessionId: string, turn: number, step: number, full: boolean) =>
    ["session-trace", org, sessionId, "step", turn, step, full] as const,
  events: (org: string | undefined, sessionId: string, turn: number, version: string) =>
    ["session-trace", org, sessionId, "events", turn, version] as const,
  request: (org: string | undefined, sessionId: string, turn: number, step: number) =>
    ["session-trace", org, sessionId, "request", turn, step] as const,
  search: (org: string | undefined, sessionId: string, q: string) =>
    ["session-trace", org, sessionId, "search", q] as const,
};

const NO_EXPANSIONS: ReadonlyMap<string, Expansion> = new Map();
/** Calls fetched per page when a batch is expanded. */
const MEMBER_PAGE = 100;
/** Search hits listed at once. */
const SEARCH_LIMIT = 30;

export function useSessionTraceOverview(sessionId: string) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: traceKeys.overview(currentOrg?.public_id, sessionId),
    queryFn: () => getSessionTrace(sessionId),
  });
}

export function useSessionTraceStep(
  sessionId: string,
  selection: { turn: number; step: number } | null,
  full = false,
) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: traceKeys.step(
      currentOrg?.public_id,
      sessionId,
      selection?.turn ?? 0,
      selection?.step ?? 0,
      full,
    ),
    queryFn: () => getSessionTraceStep(sessionId, selection!.turn, selection!.step, full),
    enabled: selection !== null,
  });
}

export interface SessionTraceTurns {
  turns: TraceTurn[];
  turnCount: number;
  loading: boolean;
  error: Error | null;
  /** Load the page before the first loaded turn. */
  loadEarlier: () => Promise<void>;
  /** Load the page after the last loaded turn. */
  loadLater: () => Promise<void>;
  /** Make a turn loaded, replacing the window when it is far away. */
  jumpTo: (turn: number) => Promise<void>;
  /** Re-read the last turns while the session runs. */
  refreshTail: () => Promise<void>;
  /** Fetch the next steps hidden behind a gap. */
  expandGap: (turn: number, gap: TraceGap) => Promise<void>;
  busy: boolean;
}

export function useSessionTraceTurns(
  sessionId: string,
  initial: TraceTurnsCursor | null,
): SessionTraceTurns {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  const [turns, setTurns] = useState<TraceTurn[]>([]);
  const [turnCount, setTurnCount] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const turnsRef = useRef(turns);
  turnsRef.current = turns;
  const queryClient = useQueryClient();
  const initialRef = useRef(initial);

  const fetchPage = useCallback(
    async (cursor: TraceTurnsCursor, replace: boolean) => {
      setBusy(true);
      try {
        const page = await listSessionTraceTurns(sessionId, { limit: PAGE, ...cursor });
        setTurnCount(page.turn_count);
        setTurns((current) => (replace ? page.turns : mergeTurns(current, page.turns)));
        setError(null);
      } catch (err) {
        setError(err instanceof Error ? err : new Error(String(err)));
      } finally {
        setBusy(false);
      }
    },
    [sessionId],
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setTurns([]);
    listSessionTraceTurns(sessionId, { limit: PAGE, ...(initialRef.current ?? {}) })
      .then((page) => {
        if (cancelled) return;
        setTurnCount(page.turn_count);
        setTurns(page.turns);
        setError(null);
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err : new Error(String(err)));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, org]);

  const loadEarlier = useCallback(async () => {
    const first = turnsRef.current[0]?.turn;
    if (first === undefined || first <= 1) return;
    await fetchPage({ before: first }, false);
  }, [fetchPage]);

  const loadLater = useCallback(async () => {
    const last = turnsRef.current[turnsRef.current.length - 1]?.turn;
    if (last === undefined) return;
    await fetchPage({ after: last }, false);
  }, [fetchPage]);

  const jumpTo = useCallback(
    async (turn: number) => {
      const loaded = turnsRef.current;
      if (loaded.some((t) => t.turn === turn)) return;
      const first = loaded[0]?.turn ?? 0;
      const last = loaded[loaded.length - 1]?.turn ?? 0;
      // Grow the window when the turn is next to it; otherwise start over
      // around the turn, so the list never holds an unbounded range.
      if (turn < first && first - turn <= PAGE) {
        await fetchPage({ before: first }, false);
      } else if (turn > last && turn - last <= PAGE) {
        await fetchPage({ after: last }, false);
      } else {
        await fetchPage({ around: turn }, true);
      }
    },
    [fetchPage],
  );

  const refreshTail = useCallback(async () => {
    const loaded = turnsRef.current;
    const last = loaded[loaded.length - 1]?.turn ?? 0;
    try {
      const page = await listSessionTraceTurns(sessionId, {
        after: Math.max(0, last - 1),
        limit: 5,
      });
      setTurnCount(page.turn_count);
      setTurns((current) => mergeTurns(current, page.turns));
      void queryClient.invalidateQueries({ queryKey: traceKeys.overview(org, sessionId) });
    } catch {
      // A failed refresh keeps what is shown; the next event retries.
    }
  }, [sessionId, org, queryClient]);

  const expandGap = useCallback(
    async (turn: number, gap: TraceGap) => {
      setBusy(true);
      try {
        const page = await listSessionTraceSteps(sessionId, turn, {
          from_step: gap.first_step,
          to_step: gap.last_step,
          limit: GAP_PAGE,
        });
        setTurns((current) =>
          current.map((t) => (t.turn === turn ? fillGap(t, gap, page.steps, page.next_step) : t)),
        );
      } finally {
        setBusy(false);
      }
    },
    [sessionId],
  );

  return {
    turns,
    turnCount,
    loading,
    error,
    loadEarlier,
    loadLater,
    jumpTo,
    refreshTail,
    expandGap,
    busy,
  };
}

/**
 * Batches and sub-agent steps opened inline. A batch shows its calls, paged; a
 * sub-agent step shows the sub-agent session's last turn, one level deep.
 */
export function useTraceExpansions(sessionId: string) {
  // Tagged with the session so opening another session starts with nothing open.
  const [state, setState] = useState<{ sessionId: string; map: ReadonlyMap<string, Expansion> }>(
    () => ({ sessionId, map: new Map() }),
  );
  const expanded = state.sessionId === sessionId ? state.map : NO_EXPANSIONS;

  const patch = useCallback(
    (key: string, update: (current?: Expansion) => Expansion | null) => {
      setState((current) => {
        const next = new Map(current.sessionId === sessionId ? current.map : undefined);
        const value = update(next.get(key));
        if (value) next.set(key, value);
        else next.delete(key);
        return { sessionId, map: next };
      });
    },
    [sessionId],
  );

  const loadBatch = useCallback(
    async (turn: number, batch: TraceBatch, from: number) => {
      const key = batchKey(turn, batch.first_step);
      patch(key, (current) => ({ steps: current?.steps ?? [], loading: true }));
      try {
        const page = await listSessionTraceSteps(sessionId, turn, {
          from_step: from,
          to_step: batch.last_step,
          limit: MEMBER_PAGE,
        });
        // A batch is a run of consecutive steps, so its range holds only its calls.
        patch(key, (current) =>
          current
            ? { steps: [...current.steps, ...page.steps], nextStep: page.next_step, loading: false }
            : null,
        );
      } catch (err) {
        patch(key, (current) =>
          current ? { ...current, loading: false, error: errorText(err) } : null,
        );
      }
    },
    [sessionId, patch],
  );

  const toggleBatch = useCallback(
    (turn: number, batch: TraceBatch) => {
      const key = batchKey(turn, batch.first_step);
      if (expanded.has(key)) patch(key, () => null);
      else void loadBatch(turn, batch, batch.first_step);
    },
    [expanded, loadBatch, patch],
  );

  const moreBatch = useCallback(
    (turn: number, batch: TraceBatch) => {
      const next = expanded.get(batchKey(turn, batch.first_step))?.nextStep;
      if (next != null) void loadBatch(turn, batch, next);
    },
    [expanded, loadBatch],
  );

  const toggleAgent = useCallback(
    async (step: TraceStep) => {
      const child = step.child_session_id;
      if (!child) return;
      const key = stepRowKey(step.turn, step.step);
      if (expanded.has(key)) {
        patch(key, () => null);
        return;
      }
      patch(key, () => ({ steps: [], loading: true, childSessionId: child }));
      try {
        // A sub-agent usually answers in one turn; show the last one inline.
        const page = await listSessionTraceTurns(child, { limit: 1 });
        const last = page.turns[page.turns.length - 1];
        const steps = (last?.items ?? []).filter(
          (i): i is TraceStep & { type: "step" } => i.type === "step",
        );
        patch(key, (current) =>
          current
            ? {
                steps,
                loading: false,
                childSessionId: child,
                hiddenTurns: Math.max(0, page.turn_count - 1),
              }
            : null,
        );
      } catch (err) {
        patch(key, (current) =>
          current ? { ...current, loading: false, error: errorText(err) } : null,
        );
      }
    },
    [expanded, patch],
  );

  return { expanded, toggleBatch, moreBatch, toggleAgent };
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Lifecycle events of the loaded, open turns while the toggle is on. */
export function useTraceLifecycle(
  sessionId: string,
  turns: TraceTurn[],
  enabled: boolean,
): ReadonlyMap<number, LifecycleEvent[]> {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  const results = useQueries({
    queries: (enabled ? turns : []).map((turn) => ({
      // A running turn is re-read as its steps grow; a finished one never changes.
      queryKey: traceKeys.events(
        org,
        sessionId,
        turn.turn,
        turn.end_sequence != null ? `end${turn.end_sequence}` : `running${turn.step_count}`,
      ),
      queryFn: async () => {
        const page = await listSessionTraceTurnEvents(sessionId, turn.turn, { limit: 500 });
        return { turn: turn.turn, events: lifecycleEvents(page.events) };
      },
      staleTime: Infinity,
    })),
  });
  return useMemo(() => {
    const map = new Map<number, LifecycleEvent[]>();
    for (const result of results) {
      if (result.data) map.set(result.data.turn, result.data.events);
    }
    return map;
    // eslint-disable-next-line react-hooks/exhaustive-deps -- results is a new array every render
  }, [results.map((r) => r.dataUpdatedAt).join(",")]);
}

/** Messages per page of the full request sheet. */
const REQUEST_PAGE = 50;

/** Every message a model call was sent, with full content, page by page. */
export function useTraceRequest(
  sessionId: string,
  selection: { turn: number; step: number } | null,
) {
  const { currentOrg } = useOrg();
  return useInfiniteQuery({
    queryKey: traceKeys.request(
      currentOrg?.public_id,
      sessionId,
      selection?.turn ?? 0,
      selection?.step ?? 0,
    ),
    queryFn: ({ pageParam }) =>
      listSessionTraceRequest(sessionId, selection!.turn, selection!.step, {
        offset: pageParam,
        limit: REQUEST_PAGE,
      }),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => {
      const loaded = pages.reduce((sum, page) => sum + page.messages.length, 0);
      return last.messages.length === REQUEST_PAGE && loaded < last.message_count
        ? loaded
        : undefined;
    },
    enabled: selection !== null,
  });
}

/** Full-text search over the session's events. */
export function useTraceSearch(sessionId: string, q: string) {
  const { currentOrg } = useOrg();
  const query = q.trim();
  return useQuery({
    queryKey: traceKeys.search(currentOrg?.public_id, sessionId, query),
    queryFn: () => searchSessionEvents(sessionId, query, SEARCH_LIMIT),
    enabled: query.length >= 2,
  });
}

/** The turn (and step, when loaded) holding an event sequence. */
export async function findTraceSequence(
  sessionId: string,
  sequence: number,
): Promise<TraceTurn | undefined> {
  const page = await listSessionTraceTurns(sessionId, { sequence, limit: 1 });
  return page.turns.find((t) => t.start_sequence <= sequence) ?? page.turns[0];
}
