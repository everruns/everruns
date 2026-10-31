"use client";

// Only the managed Agent's personal side conversations belong in Chat.

import { useEffect, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { subscribeSessionChanges } from "@/lib/session-activity";
import { listSessions } from "@/lib/api/sessions";
import { queryKeys } from "@/lib/query-keys";
import { selectChatThreads, THREAD_SCAN_LIMIT } from "@/lib/chat-threads";
import { useAgent } from "@/hooks/use-agents";
import { PLATFORM_CHAT_AGENT_NAME, PLATFORM_CHAT_STARTER_TAG } from "@/lib/chat-threads";
import { useOrg } from "@/providers/org-provider";
import type { Session } from "@/lib/api/types";

/** Observed turn/task changes refresh immediately; this polling fallback
 *  covers work outside the subscribed sessions until an authorized summary
 *  feed exists. */
const THREAD_POLL_MS = 15_000;

export interface UseChatThreadsResult {
  threads: Session[];
  total?: number;
  isLoading: boolean;
  /**
   * True once the list has actually been read. `!isLoading && !error` is not
   * the same thing: React Query reports neither while a query sits between
   * retry attempts or is enabled but has not started fetching, and `threads`
   * is an empty array in both. A caller that acts on a thread being *absent*
   * must gate on this, or a transient read failure reads as "no threads yet".
   */
  isRead: boolean;
  error: Error | null;
}

export interface UseChatThreadsOptions {
  enabled?: boolean;
  offset?: number;
  limit?: number;
  search?: string;
  /** Widen the list to archived threads too. Off by default: archiving a thread
   *  is the user asking for it to stop showing up. */
  includeArchived?: boolean;
  /** Scope threads to the console account’s linked virtual user. On by default. */
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
  const org = currentOrg?.public_id;
  const { data: platformAgent, isLoading: agentLoading } = useAgent(PLATFORM_CHAT_AGENT_NAME);
  const offset = options.offset ?? 0;
  const limit = options.limit ?? THREAD_SCAN_LIMIT;
  const enabled = !!org && !!platformAgent && (options.enabled ?? true);
  const includeArchived = options.includeArchived ?? false;
  const poll = options.poll ?? true;
  const mine = options.mine ?? true;
  const queryClient = useQueryClient();
  const queryKey = queryKeys.sessions.filtered(
    org,
    `${mine ? "my-threads" : "threads"}${includeArchived ? "+archived" : ""}:${platformAgent?.id}:${options.search ?? ""}`,
    offset,
    limit,
  );
  const queryKeyId = JSON.stringify(queryKey);
  useEffect(() => {
    if (!enabled || !poll) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const unsubscribe = subscribeSessionChanges((changedOrg) => {
      if (changedOrg !== org || timer !== undefined) return;
      // Coalesce a burst of task/turn transitions into one authorized list read.
      timer = setTimeout(() => {
        timer = undefined;
        void queryClient.invalidateQueries({ queryKey: JSON.parse(queryKeyId), exact: true });
      }, 250);
    });
    return () => {
      clearTimeout(timer);
      unsubscribe();
    };
  }, [enabled, poll, org, queryClient, queryKeyId]);

  const query = useQuery({
    // Still under the `["sessions"]` prefix, so a create/update invalidation of
    // `sessions.all()` refreshes the sidebar too. The archived variant is a
    // separate cache entry because it is a different server-side predicate.
    queryKey,
    queryFn: () =>
      listSessions({
        offset,
        limit,
        includeArchived,
        mine,
        source: "chat",
        sideChatsOnly: true,
        agentId: platformAgent?.id,
        order: "last_activity",
        search: options.search,
      }),
    enabled,
    refetchInterval: poll ? THREAD_POLL_MS : false,
  });

  const threads = useMemo(
    () =>
      selectChatThreads(query.data?.data ?? [], { includeArchived }).filter(
        (s) => !s.tags.includes(PLATFORM_CHAT_STARTER_TAG),
      ),
    [query.data, includeArchived],
  );

  return {
    threads,
    total: query.data?.total ?? 0,
    isLoading: orgLoading || agentLoading || (!!platformAgent && query.isLoading),
    isRead: query.isSuccess,
    error: (query.error as Error | null) ?? null,
  };
}
