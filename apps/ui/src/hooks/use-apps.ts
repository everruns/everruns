// App hooks for Slack bot integration
"use client";

import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { appsCrudApi, publishApp, unpublishApp } from "@/lib/api/apps";
import type { App, CreateAppRequest, UpdateAppRequest } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks } from "./create-crud-hooks";

// Keep list/detail app caches in sync so detail pages do not wait for a background refetch.
function syncAppCache(queryClient: QueryClient, app: App) {
  queryClient.setQueriesData<App | undefined>(
    { queryKey: queryKeys.apps.detail(app.id) },
    () => app,
  );
  queryClient.setQueriesData<App[] | undefined>(
    { queryKey: queryKeys.apps.all },
    (existing) =>
      existing?.map((candidate) => (candidate.id === app.id ? app : candidate)) ?? existing,
  );
}

const appCrudHooks = createCrudHooks<App, CreateAppRequest, UpdateAppRequest>({
  api: appsCrudApi,
  queryKeys: queryKeys.apps,
  syncEntityCache: syncAppCache,
});

export const useApps = appCrudHooks.useList;
export const useApp = appCrudHooks.useDetail;
export const useCreateApp = appCrudHooks.useCreate;
export const useDeleteApp = appCrudHooks.useDelete;
export const useDestroyApp = appCrudHooks.useDestroy;

export function useUpdateApp() {
  const mutation = appCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (
      variables: { appId: string; data: UpdateAppRequest },
      options?: Parameters<typeof mutation.mutate>[1],
    ) => mutation.mutate({ id: variables.appId, request: variables.data }, options),
    mutateAsync: (
      variables: { appId: string; data: UpdateAppRequest },
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: variables.appId, request: variables.data }, options),
  };
}

function useAppStatusMutation(mutationFn: (appId: string) => Promise<App>) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn,
    onSuccess: (app) => {
      syncAppCache(queryClient, app);
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(app.id) });
    },
  });
}

export function usePublishApp() {
  return useAppStatusMutation(publishApp);
}

export function useUnpublishApp() {
  return useAppStatusMutation(unpublishApp);
}
