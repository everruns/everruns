// App hooks for Slack bot integration
"use client";

import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import {
  createApp,
  deleteApp,
  getApp,
  getApps,
  publishApp,
  unpublishApp,
  updateApp,
} from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import type { App, CreateAppRequest, UpdateAppRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// Keep list/detail app caches in sync so detail pages do not wait for a background refetch.
function syncAppCache(queryClient: QueryClient, app: App) {
  queryClient.setQueriesData<App | undefined>(
    { queryKey: queryKeys.apps.detail(app.id) },
    () => app,
  );
  queryClient.setQueriesData<App[] | undefined>(
    { queryKey: queryKeys.apps.list() },
    (existing) =>
      existing?.map((candidate) => (candidate.id === app.id ? app : candidate)) ?? existing,
  );
}

export function useApps() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.apps.list(), org],
    queryFn: () => getApps(),
    enabled: !!org,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useApp(appId: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.apps.detail(appId!), org],
    queryFn: () => getApp(appId!),
    enabled: !!org && !!appId,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateAppRequest) => createApp(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
    },
  });
}

export function useUpdateApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ appId, data }: { appId: string; data: UpdateAppRequest }) =>
      updateApp(appId, data),
    onSuccess: async (app) => {
      syncAppCache(queryClient, app);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.apps.all }),
        queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(app.id) }),
      ]);
    },
  });
}

export function useDeleteApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (appId: string) => deleteApp(appId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
    },
  });
}

export function usePublishApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (appId: string) => publishApp(appId),
    onSuccess: async (app) => {
      syncAppCache(queryClient, app);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.apps.all }),
        queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(app.id) }),
      ]);
    },
  });
}

export function useUnpublishApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (appId: string) => unpublishApp(appId),
    onSuccess: async (app) => {
      syncAppCache(queryClient, app);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.apps.all }),
        queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(app.id) }),
      ]);
    },
  });
}
