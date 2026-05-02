// Harness example hooks
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listHarnessExamples, importHarnessExample } from "@/lib/api/harness-examples";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

export function useHarnessExamples() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.harnessExamples.list(), org],
    queryFn: () => listHarnessExamples(),
    enabled: !!org,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useImportHarnessExample() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (name: string) => importHarnessExample(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.harnesses.all });
    },
  });
}
