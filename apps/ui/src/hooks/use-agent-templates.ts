// Agent template hooks
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listAgentTemplates, installAgentTemplate } from "@/lib/api/agent-templates";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

export function useAgentTemplates() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.agentTemplates.list(), org],
    queryFn: () => listAgentTemplates(),
    enabled: !!org,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useInstallAgentTemplate() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (slug: string) => installAgentTemplate(slug),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    },
  });
}
