// Agent example hooks
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listAgentExamples, importAgentExample } from "@/lib/api/agent-examples";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

export function useAgentExamples() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.agentExamples.list(), org],
    queryFn: () => listAgentExamples(),
    enabled: !!org,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useImportAgentExample() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (name: string) => importAgentExample(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}
