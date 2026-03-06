// App hooks for Slack bot integration
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
import type { CreateAppRequest, UpdateAppRequest } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

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
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
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
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
    },
  });
}

export function useUnpublishApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (appId: string) => unpublishApp(appId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
    },
  });
}
