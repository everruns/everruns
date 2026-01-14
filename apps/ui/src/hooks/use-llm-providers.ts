"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getLlmProviders,
  getLlmProvider,
  createLlmProvider,
  updateLlmProvider,
  deleteLlmProvider,
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

// Provider hooks
export function useLlmProviders() {
  return useQuery({
    queryKey: queryKeys.llmProviders.list(),
    queryFn: getLlmProviders,
    staleTime: 30000,
  });
}

export function useLlmProvider(providerId: string) {
  return useQuery({
    queryKey: queryKeys.llmProviders.detail(providerId),
    queryFn: () => getLlmProvider(providerId),
    enabled: !!providerId,
  });
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
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.detail(providerId) });
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

// Model hooks
export function useLlmModels() {
  return useQuery({
    queryKey: queryKeys.llmModels.list(),
    queryFn: getLlmModels,
    staleTime: 30000,
  });
}

export function useLlmProviderModels(providerId: string) {
  return useQuery({
    queryKey: queryKeys.llmProviders.models(providerId),
    queryFn: () => getLlmProviderModels(providerId),
    enabled: !!providerId,
  });
}

export function useLlmModel(modelId: string) {
  return useQuery({
    queryKey: queryKeys.llmModels.detail(modelId),
    queryFn: () => getLlmModel(modelId),
    enabled: !!modelId,
  });
}

export function useCreateLlmModel(providerId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: CreateLlmModelRequest) => createLlmModel(providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.models(providerId) });
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
