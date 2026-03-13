// Session leased-resource hooks
"use client";

import { useQuery } from "@tanstack/react-query";
import { listSessionResources } from "@/lib/api/session-resources";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

/** List leased resources for a session */
export function useSessionResources(sessionId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.sessionResources.list(sessionId!),
    queryFn: () => listSessionResources(sessionId!),
    enabled: !!org && !!sessionId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}
