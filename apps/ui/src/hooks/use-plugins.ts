"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  marketplacesCrudApi,
  installedPluginsCrudApi,
  syncMarketplace,
  getMarketplaceCatalog,
  installPlugin,
  patchInstalledPlugin,
  updateInstalledPlugin,
} from "@/lib/api/plugins";
import type {
  CreateMarketplaceRequest,
  UpdateMarketplaceRequest,
  InstallPluginRequest,
  UpdateInstalledPluginRequest,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { createCrudHooks, useOrgScopedQuery } from "./create-crud-hooks";

// ============================================
// Marketplace hooks
// ============================================

const marketplaceCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof marketplacesCrudApi.get>>,
  CreateMarketplaceRequest,
  UpdateMarketplaceRequest
>({
  api: marketplacesCrudApi,
  queryKeys: queryKeys.pluginMarketplaces,
  staleTime: 30000,
});

export const useMarketplaces = marketplaceCrudHooks.useList;
export const useMarketplace = marketplaceCrudHooks.useDetail;
export const useCreateMarketplace = marketplaceCrudHooks.useCreate;
export const useDeleteMarketplace = marketplaceCrudHooks.useDelete;

export function useUpdateMarketplace(marketplaceId: string) {
  const mutation = marketplaceCrudHooks.useUpdate();

  return {
    ...mutation,
    mutate: (request: UpdateMarketplaceRequest, options?: Parameters<typeof mutation.mutate>[1]) =>
      mutation.mutate({ id: marketplaceId, request }, options),
    mutateAsync: (
      request: UpdateMarketplaceRequest,
      options?: Parameters<typeof mutation.mutateAsync>[1],
    ) => mutation.mutateAsync({ id: marketplaceId, request }, options),
  };
}

export function useSyncMarketplace() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => syncMarketplace(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.pluginMarketplaces.all });
    },
  });
}

// ============================================
// Marketplace catalog hook
// ============================================

export function useMarketplaceCatalog(marketplaceId: string) {
  return useOrgScopedQuery({
    queryKey: queryKeys.pluginMarketplaces.catalog(marketplaceId),
    queryFn: () => getMarketplaceCatalog(marketplaceId),
    enabled: !!marketplaceId,
    staleTime: 30000,
  });
}

// ============================================
// Installed plugin hooks
// ============================================

const installedPluginCrudHooks = createCrudHooks<
  Awaited<ReturnType<typeof installedPluginsCrudApi.get>>,
  InstallPluginRequest,
  UpdateInstalledPluginRequest
>({
  api: installedPluginsCrudApi,
  queryKeys: queryKeys.installedPlugins,
  staleTime: 30000,
});

export const useInstalledPlugins = installedPluginCrudHooks.useList;
export const useInstalledPlugin = installedPluginCrudHooks.useDetail;
export const useDeleteInstalledPlugin = installedPluginCrudHooks.useDelete;

export function useInstallPlugin() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: InstallPluginRequest) => installPlugin(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins.all });
      // Catalog entries have an `installed` flag; invalidate them too
      queryClient.invalidateQueries({ queryKey: queryKeys.pluginMarketplaces.all });
    },
  });
}

export function usePatchInstalledPlugin(pluginId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: UpdateInstalledPluginRequest) => patchInstalledPlugin(pluginId, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins.all });
    },
  });
}

export function useUpdateInstalledPlugin() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => updateInstalledPlugin(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins.all });
    },
  });
}
