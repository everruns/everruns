// Repositories a coding session works on, read from its sandbox.
"use client";

import { useQuery } from "@tanstack/react-query";
import { getSessionRepositories } from "@/lib/api/sandboxes";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

/** Read on mount and on demand: the sandbox is the source, so nothing is cached for long. */
export function useSessionRepositories(sessionId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessionRepositories.detail(sessionId!),
    queryFn: () => getSessionRepositories(sessionId!),
    enabled: !!org && !!sessionId,
    staleTime: 0,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}
