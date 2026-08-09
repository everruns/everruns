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

/** This user's chat threads, most recently active first. */
export function useChatThreads(options: { enabled?: boolean } = {}): UseChatThreadsResult {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const { user } = useAuth();
  const org = currentOrg?.public_id;
  const enabled = !!org && (options.enabled ?? true);

  const query = useQuery({
    // Same shape as the sessions list cache, so a create/update invalidation of
    // `sessions.all()` refreshes the sidebar too.
    queryKey: queryKeys.sessions.list(org, undefined, 0, THREAD_SCAN_LIMIT),
    queryFn: () => listSessions({ offset: 0, limit: THREAD_SCAN_LIMIT }),
    enabled,
    refetchInterval: THREAD_POLL_MS,
  });

  const threads = useMemo(
    () => selectChatThreads(query.data?.data ?? [], user?.id),
    [query.data, user?.id],
  );

  return {
    threads,
    isLoading: orgLoading || query.isLoading,
    error: (query.error as Error | null) ?? null,
  };
}
