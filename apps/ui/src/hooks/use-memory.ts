"use client";

import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import {
  archiveMemory,
  createMemory,
  getMemory,
  listMemories,
  syncMemoryNow,
  updateMemory,
} from "@/lib/api/memory";
import type {
  CreateMemoryRequest,
  ListMemoriesParams,
  UpdateMemoryRequest,
  Memory,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrgScopedQuery } from "./create-crud-hooks";
import { useResourceOrgFallback } from "./use-resource-org-fallback";

function syncMemoryCache(queryClient: QueryClient, memory: Memory) {
  queryClient.setQueriesData<Memory | undefined>(
    { queryKey: queryKeys.memory.detail(memory.id) },
    () => memory,
  );
  queryClient.setQueriesData<Memory[] | undefined>(
    { queryKey: queryKeys.memory.all },
    (existing) =>
      existing?.map((candidate) => (candidate.id === memory.id ? memory : candidate)) ?? existing,
  );
}

function invalidateMemoryQueries(queryClient: QueryClient, memoryId?: string) {
  queryClient.invalidateQueries({ queryKey: queryKeys.memory.all });
  if (memoryId) {
    queryClient.invalidateQueries({ queryKey: queryKeys.memory.detail(memoryId) });
  }
}

export function useMemories(options: ListMemoriesParams = {}) {
  const includeArchived = options.includeArchived ?? false;
  const search = options.search?.trim() ?? "";

  return useOrgScopedQuery({
    queryKey: queryKeys.memory.list(includeArchived, search),
    queryFn: () => listMemories({ includeArchived, search }),
  });
}

export function useMemory(memoryId: string | undefined) {
  const query = useOrgScopedQuery({
    queryKey: queryKeys.memory.detail(memoryId ?? ""),
    queryFn: () => getMemory(memoryId!),
    enabled: !!memoryId,
  });
  const fallback = useResourceOrgFallback({
    resourceId: memoryId,
    error: query.error,
    isLoading: query.isLoading,
  });
  return {
    ...query,
    isLoading: query.isLoading || fallback.isCheckingOtherOrgs,
  };
}

export function useCreateMemory() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateMemoryRequest) => createMemory(request),
    onSuccess: (memory) => {
      syncMemoryCache(queryClient, memory);
      invalidateMemoryQueries(queryClient, memory.id);
    },
  });
}

export function useUpdateMemory() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ memoryId, data }: { memoryId: string; data: UpdateMemoryRequest }) =>
      updateMemory(memoryId, data),
    onSuccess: (memory) => {
      syncMemoryCache(queryClient, memory);
      invalidateMemoryQueries(queryClient, memory.id);
    },
  });
}

export function useSyncMemory() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (memoryId: string) => syncMemoryNow(memoryId),
    onSuccess: (memory) => {
      syncMemoryCache(queryClient, memory);
      invalidateMemoryQueries(queryClient, memory.id);
    },
  });
}

export function useArchiveMemory() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (memoryId: string) => archiveMemory(memoryId),
    onSuccess: (_result, memoryId) => {
      invalidateMemoryQueries(queryClient, memoryId);
    },
  });
}
