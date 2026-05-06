"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createMemoryStore,
  forgetMemory,
  getMemoryStore,
  listMemories,
  listMemoryStores,
} from "@/lib/api/memory-stores";
import type { CreateMemoryStoreRequest, ListMemoriesParams, MemoryStore } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { useOrgScopedQuery } from "./create-crud-hooks";

export function useMemoryStores() {
  return useOrgScopedQuery<MemoryStore[]>({
    queryKey: queryKeys.memoryStores.list(),
    queryFn: () => listMemoryStores(),
  });
}

export function useMemoryStore(storeId: string | undefined) {
  return useOrgScopedQuery<MemoryStore>({
    queryKey: queryKeys.memoryStores.detail(storeId ?? ""),
    queryFn: () => getMemoryStore(storeId!),
    enabled: !!storeId,
  });
}

export function useMemories(storeId: string | undefined, params: ListMemoriesParams = {}) {
  return useOrgScopedQuery({
    queryKey: queryKeys.memoryStores.memories(storeId ?? "", params as Record<string, unknown>),
    queryFn: () => listMemories(storeId!, params),
    enabled: !!storeId,
  });
}

export function useCreateMemoryStore() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateMemoryStoreRequest) => createMemoryStore(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.memoryStores.all });
    },
  });
}

export function useForgetMemory(storeId: string | undefined) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (memoryId: string) => forgetMemory(storeId!, memoryId),
    onSuccess: () => {
      if (storeId) {
        queryClient.invalidateQueries({
          queryKey: ["memory-store", storeId, "memories"],
        });
        queryClient.invalidateQueries({
          queryKey: queryKeys.memoryStores.detail(storeId),
        });
        queryClient.invalidateQueries({ queryKey: queryKeys.memoryStores.all });
      }
    },
  });
}
