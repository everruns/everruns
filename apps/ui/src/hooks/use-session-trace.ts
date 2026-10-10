// Session Trace data: the overview through React Query, and a window of loaded
// turns grown page by page around wherever the reader is.
//
// Decision: loaded turns live in component state rather than an infinite
// query, because the window grows in both directions, jumps to arbitrary
// turns from the minimap, and refreshes only its tail while a session runs.
"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getSessionTrace,
  getSessionTraceStep,
  listSessionTraceSteps,
  listSessionTraceTurns,
  type TraceTurnsCursor,
} from "@/lib/api/session-trace";
import type { TraceGap, TraceTurn } from "@/lib/api/types";
import { fillGap, mergeTurns } from "@/components/session/trace/trace-model";
import { useOrg } from "@/providers/org-provider";

const PAGE = 20;
/** Steps fetched per "show more" on a gap. */
const GAP_PAGE = 100;

export const traceKeys = {
  overview: (org: string | undefined, sessionId: string) =>
    ["session-trace", org, sessionId, "overview"] as const,
  step: (org: string | undefined, sessionId: string, turn: number, step: number, full: boolean) =>
    ["session-trace", org, sessionId, "step", turn, step, full] as const,
};

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
