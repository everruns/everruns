"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getLlmProviders,
  getLlmProvider,
  createLlmProvider,
  updateLlmProvider,
  deleteLlmProvider,
  syncProviderModels,
  getLlmModels,
  getLlmProviderModels,
  getLlmModel,
  createLlmModel,
  updateLlmModel,
  deleteLlmModel,
} from "@/lib/api/llm-providers";
import { queryKeys } from "@/lib/query-keys";
import type {
  CreateLlmProviderRequest,
  UpdateLlmProviderRequest,
  CreateLlmModelRequest,
  UpdateLlmModelRequest,
} from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// Provider hooks
export function useLlmProviders() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.llmProviders.list(), org],
    queryFn: () => getLlmProviders(),
    enabled: !!org,
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useLlmProvider(providerId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.llmProviders.detail(providerId), org],
    queryFn: () => getLlmProvider(providerId),
    enabled: !!org && !!providerId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateLlmProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateLlmProviderRequest) => createLlmProvider(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
    },
  });
}

export function useUpdateLlmProvider(providerId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateLlmProviderRequest) =>
      updateLlmProvider(providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.llmProviders.detail(providerId),
      });
    },
  });
}

export function useDeleteLlmProvider() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (providerId: string) => deleteLlmProvider(providerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
    },
  });
}

export function useSyncProviderModels() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (providerId: string) => syncProviderModels(providerId),
    onSuccess: () => {
      // Refresh models list after sync
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
    },
  });
}

// Model hooks
export function useLlmModels() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.llmModels.list(), org],
    queryFn: () => getLlmModels(),
    enabled: !!org,
    staleTime: 30000,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useLlmProviderModels(providerId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.llmProviders.models(providerId), org],
    queryFn: () => getLlmProviderModels(providerId),
    enabled: !!org && !!providerId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useLlmModel(modelId: string) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: [...queryKeys.llmModels.detail(modelId), org],
    queryFn: () => getLlmModel(modelId),
    enabled: !!org && !!modelId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

export function useCreateLlmModel(providerId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: CreateLlmModelRequest) =>
      createLlmModel(providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
      queryClient.invalidateQueries({
        queryKey: queryKeys.llmProviders.models(providerId),
      });
    },
  });
}

export function useUpdateLlmModel(modelId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (data: UpdateLlmModelRequest) => updateLlmModel(modelId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
    },
  });
}

export function useDeleteLlmModel() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (modelId: string) => deleteLlmModel(modelId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
    },
  });
}
