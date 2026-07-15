// Session participant hooks (EVE-759, area F)
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  addSessionParticipant,
  leaveSessionParticipant,
  listSessionParticipants,
} from "@/lib/api/session-participants";
import { queryKeys } from "@/lib/query-keys";
import type { AddSessionParticipantRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

/**
 * Fetch the participants of a session. Gated on org + sessionId so single-org
 * loads don't fire before the org cookie is set.
 */
export function useSessionParticipants(sessionId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessions.participants(org, sessionId),
    queryFn: () => listSessionParticipants(sessionId!),
    enabled: !!org && !!sessionId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/** Add a member participant to a session, invalidating the participants query. */
export function useAddSessionParticipant(sessionId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (body: AddSessionParticipantRequest) => addSessionParticipant(sessionId, body),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessions.participants(org, sessionId),
      });
    },
  });
}

/** Remove a participant from a session, invalidating the participants query. */
export function useLeaveSessionParticipant(sessionId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (participantId: string) => leaveSessionParticipant(sessionId, participantId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessions.participants(org, sessionId),
      });
    },
  });
}
