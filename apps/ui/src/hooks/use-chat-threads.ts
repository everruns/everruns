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
}

/** This user's chat threads, pinned first and then ordered by recent activity. */
export function useChatThreads(options: UseChatThreadsOptions = {}): UseChatThreadsResult {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const { user } = useAuth();
  const org = currentOrg?.public_id;
  const enabled = !!org && (options.enabled ?? true);
  const includeArchived = options.includeArchived ?? false;

  const query = useQuery({
    // Still under the `["sessions"]` prefix, so a create/update invalidation of
    // `sessions.all()` refreshes the sidebar too. The archived variant is a
    // separate cache entry because it is a different server-side predicate.
    queryKey: queryKeys.sessions.filtered(
      org,
      includeArchived ? "threads+archived" : "threads",
      0,
      THREAD_SCAN_LIMIT,
    ),
    queryFn: () => listSessions({ offset: 0, limit: THREAD_SCAN_LIMIT, includeArchived }),
    enabled,
    refetchInterval: THREAD_POLL_MS,
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
