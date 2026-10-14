"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listEverrunsAppLinks, unlinkEverrunsApp } from "@/lib/api/everruns-github-app";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

export function useEverrunsAppLinks(enabled = true) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useQuery({
    queryKey: queryKeys.organizations.everrunsGithubApp(org ?? ""),
    queryFn: () => listEverrunsAppLinks(org!),
    enabled: enabled && !!org,
  });
}

export function useUnlinkEverrunsApp() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  return useMutation({
    mutationFn: (connectionId: string) => unlinkEverrunsApp(org!, connectionId),
    onSuccess: async () => {
      // Unlinking also removes every agent's choice of the installation.
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: queryKeys.organizations.everrunsGithubApp(org ?? ""),
        }),
        queryClient.invalidateQueries({ queryKey: queryKeys.agentGithub.all }),
      ]);
    },
  });
}
