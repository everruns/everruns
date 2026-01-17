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
import { useOrg } from "@/providers/org-provider";

// Provider hooks
export function useLlmProviders() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.llmProviders.list(), org],
    queryFn: () => getLlmProviders(org!),
    enabled: !!org,
    staleTime: 30000,
  });
}

export function useLlmProvider(providerId: string) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.llmProviders.detail(providerId), org],
    queryFn: () => getLlmProvider(org!, providerId),
    enabled: !!org && !!providerId,
  });
}

export function useCreateLlmProvider() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (data: CreateLlmProviderRequest) => createLlmProvider(org!, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
    },
  });
}

export function useUpdateLlmProvider(providerId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (data: UpdateLlmProviderRequest) =>
      updateLlmProvider(org!, providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.detail(providerId) });
    },
  });
}

export function useDeleteLlmProvider() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (providerId: string) => deleteLlmProvider(org!, providerId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
    },
  });
}

// Model hooks
export function useLlmModels() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.llmModels.list(), org],
    queryFn: () => getLlmModels(org!),
    enabled: !!org,
    staleTime: 30000,
  });
}

export function useLlmProviderModels(providerId: string) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.llmProviders.models(providerId), org],
    queryFn: () => getLlmProviderModels(org!, providerId),
    enabled: !!org && !!providerId,
  });
}

export function useLlmModel(modelId: string) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: [...queryKeys.llmModels.detail(modelId), org],
    queryFn: () => getLlmModel(org!, modelId),
    enabled: !!org && !!modelId,
  });
}

export function useCreateLlmModel(providerId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (data: CreateLlmModelRequest) => createLlmModel(org!, providerId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.models(providerId) });
    },
  });
}

export function useUpdateLlmModel(modelId: string) {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (data: UpdateLlmModelRequest) => updateLlmModel(org!, modelId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
    },
  });
}

export function useDeleteLlmModel() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (modelId: string) => deleteLlmModel(org!, modelId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.llmProviders.all });
    },
  });
}
