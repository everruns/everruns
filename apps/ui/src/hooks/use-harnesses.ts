// Harness hooks
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  copyHarness,
  getHarnessStats,
  harnessesCrudApi,
  previewHarness,
} from "@/lib/api/harnesses";
import type {
  CreateHarnessRequest,
  PreviewHarnessRequest,
  UpdateHarnessRequest,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks } from "./create-crud-hooks";
import { useOrg } from "@/providers/org-provider";

const harnessCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof harnessesCrudApi.get>>,
  CreateHarnessRequest,
  UpdateHarnessRequest
>({
  api: harnessesCrudApi,
  queryKeys: queryKeys.harnesses,
});

export const useHarnesses = harnessCrudHooks.useList;
export const useHarness = harnessCrudHooks.useDetail;
export const useCreateHarness = harnessCrudHooks.useCreate;
export const useDeleteHarness = harnessCrudHooks.useDelete;
export const useDestroyHarness = harnessCrudHooks.useDestroy;

export function useHarnessStats(harnessId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: queryKeys.harnesses.stats(org, harnessId),
    queryFn: () => getHarnessStats(harnessId!),
    enabled: !!org && !!harnessId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useUpdateHarness() {
  const mutation = harnessCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (
      variables: { harnessId: string; request: UpdateHarnessRequest },
      options?: Parameters<typeof mutation.mutate>[1],
    ) => mutation.mutate({ id: variables.harnessId, request: variables.request }, options),
    mutateAsync: (
      variables: { harnessId: string; request: UpdateHarnessRequest },
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: variables.harnessId, request: variables.request }, options),
  };
}

export function useCopyHarness() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: copyHarness,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.harnesses.all });
    },
  });
}

export function usePreviewHarness() {
  return useMutation({
    mutationFn: (request: PreviewHarnessRequest) => previewHarness(request),
  });
}
