"use client";

// Chat threads = ordinary sessions. See `src/lib/chat-threads.ts` for why the
// filtering is client-side today and what EVE-852 replaces it with.

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { listSessions } from "@/lib/api/sessions";
import { queryKeys } from "@/lib/query-keys";
import { selectChatThreads, THREAD_SCAN_LIMIT } from "@/lib/chat-threads";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import type { Session } from "@/lib/api/types";

/** Poll interval for the thread list. Threads gain activity from turns that run
 *  outside this tab, and there is no org-wide session event stream to subscribe
 *  to, so the list refreshes on a timer. */
const THREAD_POLL_MS = 15_000;

export interface UseChatThreadsResult {
  threads: Session[];
  isLoading: boolean;
  error: Error | null;
}

export interface UseChatThreadsOptions {
  enabled?: boolean;
  /** Widen the list to archived threads too. Off by default: archiving a thread
   *  is the user asking for it to stop showing up. */
  includeArchived?: boolean;
  /** Ask the server for this user's sessions only. The client-side owner
   *  filter runs either way; this narrows the page the server returns, so a
   *  busy org's other sessions cannot push the user's threads out of the
   *  scanned window. */
  mine?: boolean;
  /** Keep the list fresh on a timer. On by default for surfaces that display
   *  threads; a caller that only needs to read the list once (e.g. deciding
   *  whether a thread exists) turns it off so it does not add a second poll of
   *  the same endpoint. */
  poll?: boolean;
}

/** This user's chat threads, pinned first and then ordered by recent activity. */
export function useChatThreads(options: UseChatThreadsOptions = {}): UseChatThreadsResult {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const { user } = useAuth();
  const org = currentOrg?.public_id;
  const enabled = !!org && (options.enabled ?? true);
  const includeArchived = options.includeArchived ?? false;
  const poll = options.poll ?? true;
  const mine = options.mine ?? false;

  const query = useQuery({
    // Still under the `["sessions"]` prefix, so a create/update invalidation of
    // `sessions.all()` refreshes the sidebar too. The archived variant is a
    // separate cache entry because it is a different server-side predicate.
    queryKey: queryKeys.sessions.filtered(
      org,
      `${mine ? "my-threads" : "threads"}${includeArchived ? "+archived" : ""}`,
      0,
      THREAD_SCAN_LIMIT,
    ),
    queryFn: () => listSessions({ offset: 0, limit: THREAD_SCAN_LIMIT, includeArchived, mine }),
    enabled,
    refetchInterval: poll ? THREAD_POLL_MS : false,
  });

  const threads = useMemo(
    () => selectChatThreads(query.data?.data ?? [], user?.id, { includeArchived }),
    [query.data, user?.id, includeArchived],
  );

  return {
    threads,
    isLoading: orgLoading || query.isLoading,
    error: (query.error as Error | null) ?? null,
  };
}
