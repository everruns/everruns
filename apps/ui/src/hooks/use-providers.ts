"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getProviders,
  getProvider,
  getProvidersConfig,
  createProvider,
  updateProvider,
  deleteProvider,
  syncProviderModels,
  getModels,
  getProviderModels,
  getModel,
  createModel,
  updateModel,
  deleteModel,
} from "@/lib/api/providers";
import { queryKeys } from "@/lib/query-keys";
import type {
  CreateProviderRequest,
  UpdateProviderRequest,
  CreateModelRequest,
  UpdateModelRequest,
} from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// Provider hooks
export function useProviders() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.list(), org],
    queryFn: () => getProviders(),
    enabled: !!org,
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useProvider(providerId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.detail(providerId), org],
    queryFn: () => getProvider(providerId),
    enabled: !!org && !!providerId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

// Per-driver credential schemas (and caller policies), used to render the
// provider credential forms as discrete typed inputs.
export function useProvidersConfig() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.all, "config", org],
    queryFn: () => getProvidersConfig(),
    enabled: !!org,
    staleTime: 300000,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateProviderRequest) => createProvider(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
    },
  });
}

export function useUpdateProvider(providerId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateProviderRequest) => updateProvider(providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.providers.detail(providerId),
      });
    },
  });
}

export function useDeleteProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (providerId: string) => deleteProvider(providerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
    },
  });
}

export function useSyncProviderModels() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (providerId: string) => syncProviderModels(providerId),
    onSuccess: () => {
      // Refresh models list after sync
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
    },
  });
}

// Model hooks
export function useModels() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.models.list(), org],
    queryFn: () => getModels(),
    enabled: !!org,
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useProviderModels(providerId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.providers.models(providerId), org],
    queryFn: () => getProviderModels(providerId),
    enabled: !!org && !!providerId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useModel(modelId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.models.detail(modelId), org],
    queryFn: () => getModel(modelId),
    enabled: !!org && !!modelId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateModel(providerId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateModelRequest) => createModel(providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.providers.models(providerId),
      });
    },
  });
}

export function useUpdateModel(modelId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateModelRequest) => updateModel(modelId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
    },
  });
}

export function useDeleteModel() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (modelId: string) => deleteModel(modelId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
    },
  });
}
