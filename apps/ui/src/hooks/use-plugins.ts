"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  marketplacesCrudApi,
  installedPluginsCrudApi,
  syncMarketplace,
  getMarketplaceCatalog,
  getInstalledPlugins,
  installPlugin,
  patchInstalledPlugin,
  updateInstalledPlugin,
} from "@/lib/api/plugins";
import type {
  CreateMarketplaceRequest,
  UpdateMarketplaceRequest,
  InstallPluginRequest,
  InstalledPlugin,
  MarketplaceCatalogEntry,
  UpdateInstalledPluginRequest,
} from "@/lib/api/types";
import { ApiError } from "@/lib/api/client";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
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

export function useInstallPlugin(marketplaceId: string, marketplaceName: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.public_id;
  const installedListKey = [...queryKeys.installedPlugins.list(), orgId] as const;
  const catalogKey = [...queryKeys.pluginMarketplaces.catalog(marketplaceId), orgId] as const;

  return useMutation({
    mutationFn: async (request: InstallPluginRequest) => {
      try {
        return { plugin: await installPlugin(request), inventory: undefined };
      } catch (error) {
        if (!(error instanceof ApiError) || error.status !== 409) {
          throw error;
        }

        // Another tab or request may have won the install race. Reconcile from
        // the server and accept only the same plugin from the same marketplace.
        const inventory = await getInstalledPlugins();
        const plugin = inventory.find(
          (candidate) =>
            candidate.name === request.plugin_name && candidate.marketplace === marketplaceName,
        );
        if (!plugin) {
          throw error;
        }
        return { plugin, inventory };
      }
    },
    onSuccess: async ({ plugin, inventory }) => {
      if (inventory) {
        queryClient.setQueryData(installedListKey, inventory);
      } else {
        queryClient.setQueryData<InstalledPlugin[]>(installedListKey, (current) =>
          current ? [...current.filter((item) => item.id !== plugin.id), plugin] : current,
        );
      }
      queryClient.setQueryData<MarketplaceCatalogEntry[]>(catalogKey, (current) =>
        current?.map((entry) =>
          entry.name === plugin.name ? { ...entry, installed: true } : entry,
        ),
      );

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: installedListKey, exact: true }),
        queryClient.invalidateQueries({ queryKey: catalogKey, exact: true }),
      ]);
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
