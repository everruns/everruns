// Observer hooks
"use client";

import { observersCrudApi, listObserverScores } from "@/lib/api/observers";
import type { CreateObserverRequest, UpdateObserverRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks, useOrgScopedQuery } from "./create-crud-hooks";

const observerCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof observersCrudApi.get>>,
  CreateObserverRequest,
  UpdateObserverRequest
>({
  api: observersCrudApi,
  queryKeys: queryKeys.observers,
});

export const useObservers = observerCrudHooks.useList;
export const useObserver = observerCrudHooks.useDetail;
export const useCreateObserver = observerCrudHooks.useCreate;
export const useDeleteObserver = observerCrudHooks.useDelete;
export const useDestroyObserver = observerCrudHooks.useDestroy;

export function useUpdateObserver() {
  const mutation = observerCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (
      variables: { observerId: string; request: UpdateObserverRequest },
      options?: Parameters<typeof mutation.mutate>[1],
    ) => mutation.mutate({ id: variables.observerId, request: variables.request }, options),
    mutateAsync: (
      variables: { observerId: string; request: UpdateObserverRequest },
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: variables.observerId, request: variables.request }, options),
  };
}

// Trace scores hook
export function useObserverScores(observerId: string | undefined) {
  return useOrgScopedQuery({
    queryKey: queryKeys.observers.scores(observerId ?? ""),
    queryFn: () => listObserverScores(observerId!, { limit: 200 }),
    enabled: !!observerId,
  });
}
