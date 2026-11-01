// Message reaction hooks: every emoji on every message of one session.
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createContext, useContext, useEffect, useMemo, useRef, type ReactNode } from "react";
import {
  applyReaction,
  listMessageReactions,
  setMessageReaction,
  type MessageReactions,
  type ReactionSummary,
} from "@/lib/api/message-reactions";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

const EMPTY: ReactionSummary[] = [];

/**
 * Reactions keyed by message id. One request per session, not per message.
 * `version` is the newest reaction event the page has seen; when it changes
 * (someone, or the agent, reacted) the list is read again.
 */
export function useMessageReactions(
  sessionId: string | undefined,
  { enabled = true, version }: { enabled?: boolean; version?: string | null } = {},
) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  const key = queryKeys.sessions.messageReactions(org, sessionId);
  const seen = useRef(version);

  useEffect(() => {
    if (version && version !== seen.current) {
      seen.current = version;
      void queryClient.invalidateQueries({ queryKey: key });
    }
    // `key` is rebuilt every render; org and session identify it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version, org, sessionId, queryClient]);

  return useQuery({
    queryKey: key,
    queryFn: () => listMessageReactions(sessionId!),
    enabled: enabled && !!org && !!sessionId,
    select: (rows) => new Map(rows.map((row) => [row.message_id, row.reactions])),
  });
}

/** One message's reactions from the session map, stable when it has none. */
export function reactionsFor(
  map: Map<string, ReactionSummary[]> | undefined,
  messageId: string | undefined,
): ReactionSummary[] {
  return (messageId && map?.get(messageId)) || EMPTY;
}

/** Add or remove the caller's reaction. The cache updates at once and rolls back on error. */
export function useToggleMessageReaction(sessionId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const key = queryKeys.sessions.messageReactions(currentOrg?.public_id, sessionId);

  return useMutation({
    mutationFn: ({ messageId, emoji, add }: { messageId: string; emoji: string; add: boolean }) =>
      setMessageReaction(sessionId, messageId, emoji, add),
    onMutate: async ({ messageId, emoji, add }) => {
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<MessageReactions[]>(key);
      queryClient.setQueryData<MessageReactions[]>(
        key,
        applyReaction(previous ?? [], messageId, emoji, add),
      );
      return { previous };
    },
    onError: (_error, _variables, context) => {
      queryClient.setQueryData(key, context?.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: key }),
  });
}

/** What message rows need: each message's reactions and a way to toggle one. */
export interface SessionReactions {
  reactionsOf: (messageId: string | undefined) => ReactionSummary[];
  toggle: (messageId: string, emoji: string, add: boolean) => void;
}

const SessionReactionsContext = createContext<SessionReactions | null>(null);

/** The session's reactions, or `null` where reactions are not offered. */
export function useSessionReactions(): SessionReactions | null {
  return useContext(SessionReactionsContext);
}

type ReactionEventLike = { id: string; type: string };

/**
 * The newest reaction event in the stream. When it changes the reactions are
 * read again, so the agent's and other people's reactions appear live.
 */
export function latestReactionEventId(events: readonly ReactionEventLike[] | undefined) {
  const list = events ?? [];
  for (let index = list.length - 1; index >= 0; index -= 1) {
    if (list[index].type.startsWith("message.reaction.")) return list[index].id;
  }
  return null;
}

function LiveReactions({
  sessionId,
  events,
  children,
}: {
  sessionId: string;
  events: readonly ReactionEventLike[] | undefined;
  children: ReactNode;
}) {
  const version = useMemo(() => latestReactionEventId(events), [events]);
  const { data } = useMessageReactions(sessionId, { version });
  const mutation = useToggleMessageReaction(sessionId);
  const { mutate } = mutation;
  const value = useMemo<SessionReactions>(
    () => ({
      reactionsOf: (messageId) => reactionsFor(data, messageId),
      toggle: (messageId, emoji, add) => mutate({ messageId, emoji, add }),
    }),
    [data, mutate],
  );
  return (
    <SessionReactionsContext.Provider value={value}>{children}</SessionReactionsContext.Provider>
  );
}

/**
 * Offers reactions to the message rows inside it. Disabled, it reads nothing
 * and rows show no reactions (previews, playground).
 */
export function SessionReactionsScope({
  sessionId,
  enabled,
  events,
  children,
}: {
  sessionId: string;
  enabled: boolean;
  events: readonly ReactionEventLike[] | undefined;
  children: ReactNode;
}) {
  if (!enabled) return <>{children}</>;
  return (
    <LiveReactions sessionId={sessionId} events={events}>
      {children}
    </LiveReactions>
  );
}
