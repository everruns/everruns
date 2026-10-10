// Message feedback hooks: the caller's Good / Bad ratings in one session.
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  listMessageFeedback,
  setMessageFeedback,
  type MessageFeedback,
  type MessageRating,
} from "@/lib/api/message-feedback";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

/** Ratings keyed by message id. One request per session, not per message. */
export function useMessageFeedback(sessionId: string | undefined, enabled = true) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: queryKeys.sessions.messageFeedback(org, sessionId),
    queryFn: () => listMessageFeedback(sessionId!),
    enabled: enabled && !!org && !!sessionId,
    select: (rows) =>
      new Map(
        rows
          .filter((row): row is MessageFeedback & { rating: MessageRating } => row.rating !== null)
          .map((row) => [row.message_id, row.rating]),
      ),
  });
}

/** Set or clear a rating. The cache updates at once and rolls back on error. */
export function useSetMessageFeedback(sessionId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const key = queryKeys.sessions.messageFeedback(currentOrg?.public_id, sessionId);

  return useMutation({
    mutationFn: ({ messageId, rating }: { messageId: string; rating: MessageRating | null }) =>
      setMessageFeedback(sessionId, messageId, rating),
    onMutate: async ({ messageId, rating }) => {
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<MessageFeedback[]>(key);
      const others = (previous ?? []).filter((row) => row.message_id !== messageId);
      queryClient.setQueryData<MessageFeedback[]>(
        key,
        rating
          ? [
              ...others,
              {
                message_id: messageId,
                rating,
                comment: null,
                updated_at: new Date().toISOString(),
              },
            ]
          : others,
      );
      return { previous };
    },
    onError: (_error, _variables, context) => {
      queryClient.setQueryData(key, context?.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: key }),
  });
}
