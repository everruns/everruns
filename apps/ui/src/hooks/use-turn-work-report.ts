/**
 * Decisions:
 * - The server owns the one reducer that turns a turn's events into a work
 *   report (`GET /v1/sessions/{id}/work`). The UI never re-derives it; it
 *   refetches when the turn's events change, which it already streams.
 * - `revision` is anything that changes when the turn gains events (the
 *   caller passes the work-event count and whether the turn ended). It is
 *   debounced so a burst of streamed events costs one read, not one per event.
 * - Only the turn the caller names is read. Chat asks for the latest turn
 *   only: a long session has hundreds of finished turns, and their logs keep
 *   the plain "Worked for 13s" header instead of a request each.
 * - A report that has not arrived yet keeps the previous one for the same
 *   turn, so the header does not flicker back to the composed status.
 */
"use client";

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { getSessionWork } from "@/lib/api/sessions";
import type { WorkReport } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

const REFETCH_DEBOUNCE_MS = 400;

function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}

export function useTurnWorkReport(
  sessionId: string | undefined,
  turnId: string | undefined,
  revision: string,
): WorkReport | null {
  const { currentOrg } = useOrg();
  const settled = useDebounced(revision, REFETCH_DEBOUNCE_MS);
  const { data } = useQuery({
    queryKey: [...queryKeys.sessions.work(currentOrg?.public_id, sessionId, turnId), settled],
    queryFn: () => getSessionWork(sessionId as string, turnId),
    enabled: !!sessionId && !!turnId,
    staleTime: Infinity,
    retry: false,
    // A previous turn's report is dropped below by its turn id.
    placeholderData: (previous) => previous,
  });
  const report = data?.report ?? null;
  return report && report.turn_id === turnId ? report : null;
}

const LIVE_POLL_MS = 5000;

/**
 * The latest turn's report for a session shown as a row elsewhere (a run card,
 * the thread board, the sessions list), where its events are not streamed.
 * Only live rows ask, and they poll; a finished row keeps its own summary, so
 * a list of a thousand sessions costs reads only for the few still working.
 */
export function useLiveWorkReport(sessionId: string | undefined, live: boolean): WorkReport | null {
  const { currentOrg } = useOrg();
  const { data } = useQuery({
    queryKey: queryKeys.sessions.work(currentOrg?.public_id, sessionId),
    queryFn: () => getSessionWork(sessionId as string),
    enabled: !!sessionId && live,
    refetchInterval: live ? LIVE_POLL_MS : false,
    retry: false,
  });
  return live ? (data?.report ?? null) : null;
}
